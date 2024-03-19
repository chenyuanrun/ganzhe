#![allow(dead_code)]

pub mod io;
pub mod sync;

use std::{
    cell::{RefCell, UnsafeCell},
    future::Future,
    ops::Deref,
    pin::Pin,
    sync::{atomic, Arc},
    task::{ready, Poll},
};

use anyhow::anyhow;
use tokio::{
    sync::{mpsc, oneshot},
    task::LocalSet,
};

pub struct JoinHandle<T>
where
    T: Send + 'static,
{
    err: Option<Error>,
    stat: Option<JoinHandleStat<T>>,
}

enum JoinHandleStat<T>
where
    T: Send + 'static,
{
    TokioJhRx(oneshot::Receiver<tokio::task::JoinHandle<T>>),
    TokioJh(tokio::task::JoinHandle<T>),
}

impl<T> From<tokio::task::JoinHandle<T>> for JoinHandle<T>
where
    T: Send + 'static,
{
    fn from(value: tokio::task::JoinHandle<T>) -> Self {
        JoinHandle {
            err: None,
            stat: Some(JoinHandleStat::TokioJh(value)),
        }
    }
}

impl<T> From<oneshot::Receiver<tokio::task::JoinHandle<T>>> for JoinHandle<T>
where
    T: Send + 'static,
{
    fn from(value: oneshot::Receiver<tokio::task::JoinHandle<T>>) -> Self {
        JoinHandle {
            err: None,
            stat: Some(JoinHandleStat::TokioJhRx(value)),
        }
    }
}

impl<T> From<Error> for JoinHandle<T>
where
    T: Send + 'static,
{
    fn from(value: Error) -> Self {
        JoinHandle {
            err: Some(value),
            stat: None,
        }
    }
}

impl<T> Future for JoinHandle<T>
where
    T: Send + 'static,
{
    type Output = Result<T, Error>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = self.get_mut();
        if let Some(_) = &this.err {
            return Poll::Ready(Err(this.err.take().unwrap()));
        }
        assert!(this.stat.is_some());
        match this.stat.as_mut().unwrap() {
            JoinHandleStat::TokioJhRx(rx) => {
                // Poll jh rx.
                match ready!(Pin::new(rx).poll(cx)) {
                    Ok(jh) => {
                        this.stat = Some(JoinHandleStat::TokioJh(jh));
                        // Poll me again.
                        cx.waker().wake_by_ref();
                        Poll::Pending
                    }
                    Err(err) => Poll::Ready(Err(anyhow!(
                        "failed to receive jh from scheduler: {err:?}"
                    )
                    .into())),
                }
            }
            JoinHandleStat::TokioJh(jh) => match ready!(Pin::new(jh).poll(cx)) {
                Ok(output) => Poll::Ready(Ok(output)),
                Err(err) => Poll::Ready(Err(err.into())),
            },
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("IoError: {0:?}")]
    IoError(#[from] std::io::Error),
    #[error("JoinError: {0:?}")]
    JoinError(#[from] tokio::task::JoinError),
    #[error("OtherError: {0:?}")]
    OtherError(#[from] anyhow::Error),
}

#[derive(Debug, Default)]
pub struct Builder {
    cross_scheduler_threads_count: Option<usize>,
    local_scheduler_threads_count: Option<usize>,
}

impl Builder {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn cross_scheduler_threads(mut self, count: usize) -> Self {
        assert!(count > 0);
        self.cross_scheduler_threads_count = Some(count);
        self
    }

    pub fn local_scheduler_threads(mut self, count: usize) -> Self {
        assert!(count > 0);
        self.local_scheduler_threads_count = Some(count);
        self
    }

    pub fn build(self) -> Result<Runtime, Error> {
        let default_count: usize = std::thread::available_parallelism()?.into();
        let cross_scheduler_threads_count =
            self.cross_scheduler_threads_count.unwrap_or(default_count);
        let local_scheduler_threads_count =
            self.local_scheduler_threads_count.unwrap_or(default_count);
        let runtime_context = Arc::new(RuntimeContext::default());

        let runtime_context_clone = runtime_context.clone();
        let cross_thread_id = Arc::new(atomic::AtomicUsize::new(0));
        let cross_scheduler = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name_fn({
                move || {
                    let id = cross_thread_id.fetch_add(1, atomic::Ordering::AcqRel);
                    format!("cross-{id}")
                }
            })
            .on_thread_start(move || {
                let runtime_context_clone = runtime_context_clone.clone();
                CONTEXT.with_borrow_mut(move |context| {
                    *context = Some(runtime_context_clone);
                });

                THREAD_INFO.with_borrow_mut(|context| {
                    *context = Some(ThreadInfo {
                        thread_type: ThreadType::Cross,
                    });
                });
            })
            .on_thread_stop(|| {
                CONTEXT.with_borrow_mut(|context| {
                    context.take();
                })
            })
            .worker_threads(cross_scheduler_threads_count)
            .build()?;
        let mut local_schedulers = Vec::new();
        for id in 0..local_scheduler_threads_count {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let handle = rt.handle().clone();
            let (msg_tx, mut msg_rx) = mpsc::unbounded_channel::<RtMsg>();
            let runtime_context_clone = runtime_context.clone();
            let thread_jh = std::thread::Builder::new()
                .name(format!("local-{id}"))
                .spawn(move || {
                    CONTEXT.with_borrow_mut(|context| {
                        *context = Some(runtime_context_clone);
                    });
                    THREAD_INFO.with_borrow_mut(|context| {
                        *context = Some(ThreadInfo {
                            thread_type: ThreadType::Local { index: id },
                        });
                    });

                    rt.block_on(async move {
                        let local_set = LocalSet::new();
                        local_set
                            .run_until(async move {
                                while let Some(msg) = msg_rx.recv().await {
                                    // handle msg here.
                                    match msg {
                                        RtMsg::SpawnTask(mut spawner) => spawner.spawn(),
                                    }
                                }
                            })
                            .await;
                    });
                    CONTEXT.with_borrow_mut(|context| {
                        context.take();
                    });
                })?;
            local_schedulers.push(LocalSchedulerHandle {
                id,
                msg_tx,
                handle,
                thread_jh: Some(thread_jh),
            });
        }

        let runtime_inner = Arc::new(RuntimeInner {
            cross_scheduler,
            local_schedulers,
        });
        // safety: there is no task accessing CONTEXT now.
        unsafe {
            runtime_context.set_runtime_inner(runtime_inner.clone());
        };
        Ok(Runtime {
            inner: runtime_inner,
        })
    }
}

thread_local! {
    static CONTEXT: RefCell<Option<Arc<RuntimeContext>>> = RefCell::new(None);
    static THREAD_INFO: RefCell<Option<ThreadInfo>> = RefCell::new(None);
}

#[derive(Default)]
struct RuntimeContext {
    inner: UnsafeCell<Option<Arc<RuntimeInner>>>,
}

unsafe impl Send for RuntimeContext {}
unsafe impl Sync for RuntimeContext {}

impl RuntimeContext {
    unsafe fn set_runtime_inner(&self, runtime_inner: Arc<RuntimeInner>) {
        *self.inner.get() = Some(runtime_inner);
    }

    unsafe fn unset_runtime_inner(&self) {
        (&mut *self.inner.get()).take();
    }

    fn runtime_inner(&self) -> &Arc<RuntimeInner> {
        // safety: runtime is initialized.
        unsafe { (*self.inner.get()).as_ref().unwrap() }
    }
}

#[derive(Debug, Clone)]
pub enum ThreadType {
    Cross,
    Local { index: usize },
    Foreign,
}

#[derive(Debug, Clone)]
struct ThreadInfo {
    thread_type: ThreadType,
}

enum RtMsg {
    SpawnTask(Box<dyn TaskSpawner>),
}

trait TaskSpawner: Send + 'static {
    fn spawn(&mut self);
}

struct LocalTaskSpawner<F, Fut>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future + 'static,
    Fut::Output: Send + 'static,
{
    spawner: Option<F>,
    jh_tx: Option<oneshot::Sender<tokio::task::JoinHandle<Fut::Output>>>,
}

impl<F, Fut> TaskSpawner for LocalTaskSpawner<F, Fut>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future + 'static,
    Fut::Output: Send + 'static,
{
    fn spawn(&mut self) {
        let spawner = self.spawner.take().unwrap();
        let jh_tx = self.jh_tx.take().unwrap();

        let future = spawner();
        let jh = tokio::task::spawn_local(future);
        let _ = jh_tx.send(jh);
    }
}

// TODO:
// - stop runtime;
#[derive(Debug)]
pub struct Runtime {
    inner: Arc<RuntimeInner>,
}

impl Deref for Runtime {
    type Target = RuntimeInner;
    fn deref(&self) -> &Self::Target {
        self.inner.as_ref()
    }
}

impl Runtime {
    pub fn block_on<F>(&self, future: F) -> F::Output
    where
        F: Future,
    {
        self.inner.block_on(future)
    }
}

#[derive(Debug)]
pub struct RuntimeInner {
    cross_scheduler: tokio::runtime::Runtime,
    local_schedulers: Vec<LocalSchedulerHandle>,
}

impl RuntimeInner {
    fn choose_local_scheduler(&self, index: usize) -> &LocalSchedulerHandle {
        &self.local_schedulers[index % self.local_schedulers.len()]
    }

    pub fn block_on<F>(self: &Arc<Self>, future: F) -> F::Output
    where
        F: Future,
    {
        let mut is_foreign = false;
        CONTEXT.with_borrow_mut(|context| {
            if context.is_none() {
                is_foreign = true;
            }
            let runtime_context = Arc::new(RuntimeContext::default());
            // safety: no one share runtime_context.
            unsafe {
                runtime_context.set_runtime_inner(self.clone());
            }
            *context = Some(runtime_context);
        });
        THREAD_INFO.with_borrow_mut(|info| {
            if info.is_none() {
                *info = Some(ThreadInfo {
                    thread_type: ThreadType::Foreign,
                });
            }
        });

        let res = self.cross_scheduler.block_on(future);

        if is_foreign {
            CONTEXT.with_borrow_mut(|context| {
                context.take();
            });
            THREAD_INFO.with_borrow_mut(|info| {
                info.take();
            });
        }

        res
    }

    pub fn spawn_to_cross_scheduler<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.cross_scheduler.spawn(future).into()
    }

    pub fn spawn_to_local_scheduler<F, Fut>(&self, index: usize, f: F) -> JoinHandle<Fut::Output>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future + 'static,
        Fut::Output: Send + 'static,
    {
        let (jh_tx, jh_rx) = oneshot::channel();
        let spawner = LocalTaskSpawner {
            spawner: Some(f),
            jh_tx: Some(jh_tx),
        };
        let rt_msg = RtMsg::SpawnTask(Box::new(spawner));
        let scheduler = self.choose_local_scheduler(index);
        if let Err(_) = scheduler.msg_tx.send(rt_msg) {
            return Error::from(anyhow!("failed to send task to {index} local scheduler")).into();
        }
        jh_rx.into()
    }

    pub fn spawn_to_random_local_scheduler<F, Fut>(&self, f: F) -> JoinHandle<Fut::Output>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future + 'static,
        Fut::Output: Send + 'static,
    {
        let index: usize = rand::random();
        self.spawn_to_local_scheduler(index, f)
    }
}

#[derive(Debug)]
struct LocalSchedulerHandle {
    id: usize,
    msg_tx: mpsc::UnboundedSender<RtMsg>,
    handle: tokio::runtime::Handle,
    thread_jh: Option<std::thread::JoinHandle<()>>,
}

pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::spawn(future).into()
}

pub fn spawn_local<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + 'static,
    F::Output: Send + 'static,
{
    tokio::task::spawn_local(future).into()
}

pub fn spawn_to_cross_scheduler<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    CONTEXT.with_borrow(move |context| {
        context
            .as_ref()
            .unwrap()
            .runtime_inner()
            .spawn_to_cross_scheduler(future)
    })
}

pub fn spawn_to_local_scheduler<F, Fut>(index: usize, f: F) -> JoinHandle<Fut::Output>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future + 'static,
    Fut::Output: Send + 'static,
{
    CONTEXT.with_borrow(move |context| {
        context
            .as_ref()
            .unwrap()
            .runtime_inner()
            .spawn_to_local_scheduler(index, f)
    })
}

pub fn spawn_to_random_local_scheduler<F, Fut>(f: F) -> JoinHandle<Fut::Output>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future + 'static,
    Fut::Output: Send + 'static,
{
    CONTEXT.with_borrow(move |context| {
        context
            .as_ref()
            .unwrap()
            .runtime_inner()
            .spawn_to_random_local_scheduler(f)
    })
}

pub fn thread_type() -> Option<ThreadType> {
    THREAD_INFO.with_borrow(|context| context.clone().map(|info| info.thread_type))
}
