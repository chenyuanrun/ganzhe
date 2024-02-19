#![allow(dead_code)]

use std::{
    cell::RefCell,
    future::Future,
    ops::Deref,
    pin::Pin,
    rc::Rc,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};

type IoResult<T> = std::io::Result<T>;

struct AsyncResultArc<T>
where
    T: Send + 'static,
{
    inner: Arc<Mutex<AsyncResult<T>>>,
}

impl<T> Clone for AsyncResultArc<T>
where
    T: Send + 'static,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

struct AsyncResultRc<T>
where
    T: Send + 'static,
{
    inner: Rc<RefCell<AsyncResult<T>>>,
}

impl<T> Clone for AsyncResultRc<T>
where
    T: Send + 'static,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

#[derive(Default)]
struct AsyncResult<T>
where
    T: Send + 'static,
{
    done: bool,
    result: Option<IoResult<T>>,
    waker: Option<Waker>,
}

enum TaskAsyncResult<T>
where
    T: Send + 'static,
{
    CrossThread(AsyncResultArc<T>),
    LocalThread(AsyncResultRc<T>),
    Direct(AsyncResult<T>),
}

macro_rules! handle_async_result {
    ($guard:expr, $cx:expr) => {
        if $guard.done {
            assert!($guard.result.is_some());
            Poll::Ready($guard.result.take().unwrap())
        } else if $guard.waker.is_none() || !$cx.waker().will_wake($guard.waker.as_ref().unwrap()) {
            $guard.waker = Some($cx.waker().clone());
            Poll::Pending
        } else {
            Poll::Pending
        }
    };
}

impl<T> TaskAsyncResult<T>
where
    T: Send + 'static,
{
    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<IoResult<T>> {
        match self {
            TaskAsyncResult::CrossThread(result) => {
                let mut guard = result.inner.lock().unwrap();
                handle_async_result!(guard, cx)
            }
            TaskAsyncResult::LocalThread(result) => {
                let mut guard = result.inner.deref().borrow_mut();
                handle_async_result!(guard, cx)
            }
            TaskAsyncResult::Direct(result) => {
                handle_async_result!(result, cx)
            }
        }
    }
}

pub trait Image {
    fn stat(&self) -> ImageStat;
    fn read<'a>(&self, offset: usize, buf: &'a mut [u8]) -> ImageRead<'a>;
    fn write<'a>(&self, offset: usize, buf: &'a [u8]) -> ImageWrite<'a>;
    fn write_zero(&self, offset: usize, length: usize) -> ImageWriteZero<'_>;
    fn flush(&self) -> ImageFlush<'_>;
    fn trim(&self, offset: usize, length: usize) -> ImageTrim<'_>;
    fn dup(&self) -> Box<dyn Image>;
}

pub struct ImageStat {
    size: usize,
}

pub struct ImageRead<'a> {
    image: &'a dyn Image,
    result: TaskAsyncResult<usize>,
    offset: usize,
    buf: &'a mut [u8],
}

impl<'a> Future for ImageRead<'a> {
    type Output = std::io::Result<usize>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        this.result.poll(cx)
    }
}

pub struct ImageWrite<'a> {
    image: &'a dyn Image,
    result: TaskAsyncResult<usize>,
    offset: usize,
    buf: &'a mut [u8],
}

impl<'a> Future for ImageWrite<'a> {
    type Output = std::io::Result<usize>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        this.result.poll(cx)
    }
}

pub struct ImageFlush<'a> {
    image: &'a dyn Image,
    result: TaskAsyncResult<()>,
}

impl<'a> Future for ImageFlush<'a> {
    type Output = std::io::Result<()>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        this.result.poll(cx)
    }
}

pub struct ImageTrim<'a> {
    image: &'a dyn Image,
    result: TaskAsyncResult<()>,
    offset: usize,
    length: usize,
}

impl<'a> Future for ImageTrim<'a> {
    type Output = std::io::Result<()>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        this.result.poll(cx)
    }
}

pub struct ImageWriteZero<'a> {
    image: &'a dyn Image,
    result: TaskAsyncResult<()>,
    offset: usize,
    length: usize,
}

impl<'a> Future for ImageWriteZero<'a> {
    type Output = std::io::Result<()>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        this.result.poll(cx)
    }
}
