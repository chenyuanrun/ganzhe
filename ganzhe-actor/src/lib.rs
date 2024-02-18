#![allow(unused_variables, async_fn_in_trait, dead_code)]

use std::{
    cell::RefCell,
    future::Future,
    pin::Pin,
    rc::Rc,
    sync::Arc,
    task::{ready, Poll},
};

use anyhow::anyhow;
use ganzhe_rt::{sync::Stat, JoinHandle};
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("IoError: {0:?}")]
    IoError(#[from] std::io::Error),
    #[error("RtError: {0:?}")]
    RtError(#[from] ganzhe_rt::Error),
    #[error("OtherError: {0:?}")]
    OtherError(#[from] anyhow::Error),
}

pub trait Actor: Sized + 'static {
    fn on_start(ctx: &Context<Self>) {}
    fn on_stop(ctx: &Context<Self>) {}
}

type ChannelBoxTx<A> = mpsc::UnboundedSender<Box<dyn ChannelBox<A>>>;
type ChannelBoxRx<A> = mpsc::UnboundedReceiver<Box<dyn ChannelBox<A>>>;

struct ChannelRx<A: Actor> {
    tx: ChannelBoxTx<A>,
    rx: ChannelBoxRx<A>,
    tx_count: Arc<()>,
}

impl<A: Actor> ChannelRx<A> {
    fn new_tx(&self) -> ChannelTx<A> {
        ChannelTx {
            tx: self.tx.clone(),
            tx_count: self.tx_count.clone(),
        }
    }

    fn tx_count(&self) -> usize {
        Arc::strong_count(&self.tx_count)
    }
}

struct ChannelTx<A: Actor> {
    tx: ChannelBoxTx<A>,
    tx_count: Arc<()>,
}

impl<A: Actor> Clone for ChannelTx<A> {
    fn clone(&self) -> Self {
        ChannelTx {
            tx: self.tx.clone(),
            tx_count: self.tx_count.clone(),
        }
    }
}

fn actor_channel<A: Actor>() -> (ChannelTx<A>, ChannelRx<A>) {
    let tx_count = Arc::new(());
    let (tx, rx) = mpsc::unbounded_channel();
    (
        ChannelTx::<A> {
            tx: tx.clone(),
            tx_count: tx_count.clone(),
        },
        ChannelRx::<A> {
            tx: tx.clone(),
            rx,
            tx_count: tx_count.clone(),
        },
    )
}

trait ChannelBox<A: Actor>: Send + 'static {
    fn unbox(&mut self, ctx: &Context<A>);
}

struct MessageEnvelope<M, R>
where
    M: Send + Sized + 'static,
    R: Send + Sized + 'static,
{
    msg: Option<M>,
    resp_tx: Option<oneshot::Sender<R>>,
}

impl<A, M, R> ChannelBox<A> for MessageEnvelope<M, R>
where
    A: Actor + Handler<M, Resp = R>,
    M: Send + Sized + 'static,
    R: Send + Sized + 'static,
{
    fn unbox(&mut self, ctx: &Context<A>) {
        let msg = self.msg.take().unwrap();
        let resp_tx = self.resp_tx.take();
        if <A as Handler<M>>::can_fast_handle(&msg, ctx) {
            let resp = <A as Handler<M>>::fast_handle(msg, ctx);
            if let Some(resp_tx) = resp_tx {
                let _ = resp_tx.send(resp);
            }
        } else {
            let ctx = ctx.clone();
            ganzhe_rt::spawn_local(async move {
                let resp = <A as Handler<M>>::handle(msg, &ctx).await;
                if let Some(resp_tx) = resp_tx {
                    let _ = resp_tx.send(resp);
                }
            });
        }
    }
}

pub struct ActorSpawner<A: Actor> {
    channel: (ChannelTx<A>, ChannelRx<A>),
}

impl<A: Actor> ActorSpawner<A> {
    pub fn new() -> Self {
        Self {
            channel: actor_channel(),
        }
    }

    pub fn addr(&self) -> Addr<A> {
        Addr {
            channel_tx: self.channel.0.clone(),
        }
    }

    pub fn spawn(self, actor: A) -> Addr<A> {
        assert!(matches!(
            ganzhe_rt::thread_type(),
            Some(ganzhe_rt::ThreadType::Local { index })
        ));
        let addr = self.addr();
        let context = Context {
            actor: Rc::new(RefCell::new(actor)),
            channel_tx: Rc::new(self.channel.0.clone()),
            stat: Rc::new(ganzhe_rt::sync::Stat::new(ActorStat::Creating)),
        };
        ganzhe_rt::spawn_local(async move {
            context.msg_loop(self.channel.1).await;
        });
        addr
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActorStat {
    Creating,
    Running,
    Stopped,
}

pub struct Context<A: Actor> {
    actor: Rc<RefCell<A>>,
    channel_tx: Rc<ChannelTx<A>>,
    stat: Rc<Stat<ActorStat>>,
}

impl<A: Actor> Clone for Context<A> {
    fn clone(&self) -> Self {
        Self {
            actor: self.actor.clone(),
            channel_tx: self.channel_tx.clone(),
            stat: self.stat.clone(),
        }
    }
}

impl<A: Actor> Context<A> {
    pub fn addr(&self) -> Addr<A> {
        Addr {
            channel_tx: (*self.channel_tx).clone(),
        }
    }

    pub fn actor(&self) -> &RefCell<A> {
        &self.actor
    }

    pub fn actor_stat(&self) -> &Rc<Stat<ActorStat>> {
        &self.stat
    }

    pub fn spawn_task<F>(&self, f: F) -> JoinHandle<F::Output>
    where
        F: Future + 'static,
        F::Output: Send + 'static,
    {
        ganzhe_rt::spawn_local(f)
    }

    async fn msg_loop(self, mut channel_rx: ChannelRx<A>) {
        A::on_start(&self);
        self.stat.update_stat(ActorStat::Running);
        while let Some(mut msg) = channel_rx.rx.recv().await {
            msg.unbox(&self);
            if matches!(self.stat.current_stat(), ActorStat::Stopped) {
                break;
            }
        }
        self.stat.update_stat(ActorStat::Stopped);
        A::on_stop(&self);
    }
}

pub struct Addr<A: Actor> {
    channel_tx: ChannelTx<A>,
}

impl<A: Actor> Clone for Addr<A> {
    fn clone(&self) -> Self {
        Self {
            channel_tx: self.channel_tx.clone(),
        }
    }
}

impl<A: Actor> Addr<A> {
    pub fn send<M>(&self, msg: M) -> WaitForResp<<A as Handler<M>>::Resp>
    where
        M: Send + Sized + 'static,
        A: Handler<M>,
    {
        let (resp_tx, resp_rx) = oneshot::channel::<<A as Handler<M>>::Resp>();
        let channel_box: Box<dyn ChannelBox<A>> = Box::new(MessageEnvelope {
            msg: Some(msg),
            resp_tx: Some(resp_tx),
        });
        let error = self
            .channel_tx
            .tx
            .send(channel_box)
            .map_err(|_| Error::from(anyhow!("failed to send msg to channel")))
            .err();

        WaitForResp { error, resp_rx }
    }

    pub fn do_send<M>(&self, msg: M) -> Result<(), Error>
    where
        M: Send + Sized + 'static,
        A: Handler<M>,
    {
        let channel_box: Box<dyn ChannelBox<A>> = Box::new(MessageEnvelope {
            msg: Some(msg),
            resp_tx: None,
        });
        self.channel_tx
            .tx
            .send(channel_box)
            .map_err(|_| anyhow!("failed to send msg to channel"))?;
        Ok(())
    }

    pub fn msg_channel<M>(&self) -> MsgChannel<M, <A as Handler<M>>::Resp>
    where
        M: Send + Sized + 'static,
        A: Handler<M>,
    {
        MsgChannel {
            messenger: Box::new(self.clone()),
        }
    }
}

pub struct WaitForResp<R>
where
    R: Send + Sized + 'static,
{
    error: Option<Error>,
    resp_rx: oneshot::Receiver<R>,
}

impl<R> Future for WaitForResp<R>
where
    R: Send + Sized + 'static,
{
    type Output = Result<R, Error>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = self.get_mut();
        if let Some(error) = this.error.take() {
            return Poll::Ready(Err(error));
        }
        Poll::Ready(
            ready!(Pin::new(&mut this.resp_rx).poll(cx))
                .map_err(|_| anyhow!("failed to send msg to channel").into()),
        )
    }
}

pub trait Handler<M>
where
    Self: Actor,
    M: Send + Sized + 'static,
{
    type Resp: Send + Sized + 'static;

    fn can_fast_handle(msg: &M, ctx: &Context<Self>) -> bool {
        false
    }

    fn fast_handle(msg: M, ctx: &Context<Self>) -> Self::Resp {
        unimplemented!("Need to be implemented if can_fast_handle return true")
    }

    async fn handle(msg: M, ctx: &Context<Self>) -> Self::Resp {
        unimplemented!("Need to be implemented")
    }
}

pub enum ActorControl {
    Stop,
}

impl<A> Handler<ActorControl> for A
where
    A: Actor,
{
    type Resp = ();

    fn can_fast_handle(_msg: &ActorControl, _ctx: &Context<Self>) -> bool {
        true
    }

    fn fast_handle(msg: ActorControl, ctx: &Context<Self>) -> Self::Resp {
        match msg {
            ActorControl::Stop => {
                ctx.stat.update_stat(ActorStat::Stopped);
            }
        }
    }
}

pub struct MsgChannel<M, R>
where
    M: Send + Sized + 'static,
    R: Send + Sized + 'static,
{
    messenger: Box<dyn Messenger<M, R>>,
}

impl<M, R> MsgChannel<M, R>
where
    M: Send + Sized + 'static,
    R: Send + Sized + 'static,
{
    pub fn send(&self, msg: M) -> WaitForResp<R> {
        self.messenger.send(msg)
    }

    pub fn do_send(&self, msg: M) -> Result<(), Error> {
        self.messenger.do_send(msg)
    }
}

impl<M, R> Clone for MsgChannel<M, R>
where
    M: Send + Sized + 'static,
    R: Send + Sized + 'static,
{
    fn clone(&self) -> Self {
        Self {
            messenger: self.messenger.dup(),
        }
    }
}

trait Messenger<M, R>
where
    M: Send + Sized + 'static,
    R: Send + Sized + 'static,
{
    fn send(&self, msg: M) -> WaitForResp<R>;
    fn do_send(&self, msg: M) -> Result<(), Error>;
    fn dup(&self) -> Box<dyn Messenger<M, R>>;
}

impl<A, M> Messenger<M, <A as Handler<M>>::Resp> for Addr<A>
where
    M: Send + Sized + 'static,
    A: Handler<M>,
{
    fn send(&self, msg: M) -> WaitForResp<<A as Handler<M>>::Resp> {
        self.send(msg)
    }

    fn do_send(&self, msg: M) -> Result<(), Error> {
        self.do_send(msg)
    }

    fn dup(&self) -> Box<dyn Messenger<M, <A as Handler<M>>::Resp>> {
        Box::new(self.clone())
    }
}
