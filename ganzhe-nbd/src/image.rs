#![allow(dead_code)]

use std::{cell::RefCell, rc::Rc};

use async_trait::async_trait;

type IoResult<T> = std::io::Result<T>;

// The implementation of Image should be cancellation-safe: if the future is
// dropped, the passed-in buf should not be held anymore.
#[async_trait(?Send)]
pub trait Image: 'static {
    fn stat(&self) -> ImageStat;
    async fn read(&self, offset: usize, buf: &mut [u8]) -> IoResult<usize>;
    async fn write(&self, offset: usize, buf: &[u8]) -> IoResult<usize>;
    async fn write_zero(&self, offset: usize, length: usize) -> IoResult<usize>;
    async fn flush(&self) -> IoResult<()>;
    async fn trim(&self, offset: usize, length: usize) -> IoResult<()>;
}

pub struct ImageStat {
    size: usize,
}

#[derive(Clone)]
pub struct MemImage {
    inner: Rc<RefCell<MemImageInner>>,
}

impl MemImage {
    pub fn new(size: usize) -> IoResult<Self> {
        let mut buf = Vec::with_capacity(size);
        unsafe { buf.set_len(size) };
        Ok(Self {
            inner: Rc::new(RefCell::new(MemImageInner { size, buf })),
        })
    }
}

struct MemImageInner {
    size: usize,
    buf: Vec<u8>,
}

#[async_trait(?Send)]
impl Image for MemImage {
    fn stat(&self) -> ImageStat {
        ImageStat {
            size: self.inner.borrow().size,
        }
    }

    async fn read(&self, offset: usize, buf: &mut [u8]) -> IoResult<usize> {
        let mut length = buf.len();
        let inner = self.inner.borrow_mut();
        if offset + length > inner.size {
            length = inner.size - offset;
        }
        unsafe {
            std::ptr::copy_nonoverlapping(
                inner.buf[offset..offset + length].as_ptr(),
                buf.as_mut_ptr(),
                length,
            );
        }
        Ok(length)
    }

    async fn write(&self, offset: usize, buf: &[u8]) -> IoResult<usize> {
        let mut length = buf.len();
        let mut inner = self.inner.borrow_mut();
        if offset + length > inner.size {
            length = inner.size - offset;
        }
        unsafe {
            std::ptr::copy_nonoverlapping(
                buf.as_ptr(),
                (&mut inner.buf[offset..offset + length]).as_mut_ptr(),
                length,
            );
        }
        Ok(length)
    }

    async fn write_zero(&self, offset: usize, mut length: usize) -> IoResult<usize> {
        let mut inner = self.inner.borrow_mut();
        if offset + length > inner.size {
            length = inner.size - offset;
        }
        unsafe {
            std::ptr::write_bytes(
                (&mut inner.buf[offset..offset + length]).as_mut_ptr(),
                0,
                length,
            );
        }
        Ok(length)
    }

    async fn flush(&self) -> IoResult<()> {
        Ok(())
    }

    async fn trim(&self, offset: usize, length: usize) -> IoResult<()> {
        self.write_zero(offset, length).await?;
        Ok(())
    }
}

pub struct DropGuard<F>
where
    F: FnOnce(),
{
    guard: Option<F>,
}

impl<F> DropGuard<F>
where
    F: FnOnce(),
{
    pub fn new(f: F) -> Self {
        Self { guard: Some(f) }
    }

    pub fn disarm(mut self) {
        self.guard.take();
    }
}

impl<F> Drop for DropGuard<F>
where
    F: FnOnce(),
{
    fn drop(&mut self) {
        if let Some(guard) = self.guard.take() {
            guard()
        }
    }
}

#[async_trait]
pub trait ImageOpener: Send + 'static {
    async fn open(self: Box<Self>) -> Rc<dyn Image>;
}

#[async_trait]
pub trait ImageProvider: Send + 'static {
    fn name(&self) -> &'static str;
    async fn open_image<'a>(&self, desc: &'a str) -> IoResult<Box<dyn ImageOpener>>;
}
