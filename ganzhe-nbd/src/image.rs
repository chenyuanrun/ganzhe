#![allow(dead_code)]

use std::{
    cell::{Ref, RefCell},
    rc::Rc,
};

use async_trait::async_trait;

type IoResult<T> = std::io::Result<T>;

#[async_trait(?Send)]
pub trait Image {
    fn stat(&self) -> ImageStat;
    async fn read(&self, offset: usize, buf: &mut [u8]) -> IoResult<usize>;
    async fn write(&self, offset: usize, buf: &[u8]) -> IoResult<usize>;
    async fn write_zero(&self, offset: usize, length: usize) -> IoResult<usize>;
    async fn flush(&self) -> IoResult<()>;
    async fn trim(&self, offset: usize, length: usize) -> IoResult<()>;
    fn dup(&self) -> Box<dyn Image>;
}

pub struct ImageStat {
    size: usize,
}

pub trait ImageProvider {}

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

    fn dup(&self) -> Box<dyn Image> {
        Box::new(self.clone())
    }
}
