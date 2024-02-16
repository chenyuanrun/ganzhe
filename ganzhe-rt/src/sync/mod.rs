use std::{
    cell::RefCell,
    future::Future,
    marker::PhantomData,
    task::{Poll, Waker},
};

struct UnSendUnSync(*const ());

pub struct Stat<T>
where
    T: Clone + Eq + 'static,
{
    stat: RefCell<T>,
    wakers: RefCell<Vec<Waker>>,
    _p: PhantomData<UnSendUnSync>,
}

impl<T> Stat<T>
where
    T: Clone + Eq + 'static,
{
    pub fn new(value: T) -> Self {
        Self {
            stat: RefCell::new(value),
            wakers: RefCell::new(Vec::new()),
            _p: Default::default(),
        }
    }

    pub fn current_stat(&self) -> T {
        self.stat.borrow().clone()
    }

    pub fn update_stat(&self, value: T) {
        if self.current_stat() == value {
            return;
        }
        *self.stat.borrow_mut() = value;
        let mut wakers = self.wakers.borrow_mut();
        for waker in wakers.drain(..) {
            waker.wake();
        }
    }

    pub fn changed(&self) -> Changed<'_, T> {
        Changed {
            stat: self,
            waiting: false,
        }
    }

    pub async fn wait_for(&self, mut f: impl FnMut(&T) -> bool) -> T {
        loop {
            let cur = self.current_stat();
            if f(&cur) {
                return cur;
            }
            self.changed().await;
        }
    }

    pub async fn wait_for_value(&self, value: &T) {
        self.wait_for(|cur| cur == value).await;
    }
}

pub struct Changed<'a, T>
where
    T: Clone + Eq + 'static,
{
    stat: &'a Stat<T>,
    waiting: bool,
}

impl<'a, T> Future for Changed<'a, T>
where
    T: Clone + Eq + 'static,
{
    type Output = ();
    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = self.get_mut();
        if this.waiting {
            this.waiting = false;
            return Poll::Ready(());
        }
        this.stat.wakers.borrow_mut().push(cx.waker().to_owned());
        this.waiting = true;
        Poll::Pending
    }
}
