#![allow(dead_code)]

use async_trait::async_trait;

type IoResult<T> = std::io::Result<T>;

#[async_trait(?Send)]
pub trait Image {
    fn stat(&self) -> ImageStat;
    async fn read(&self, offset: usize, buf: &mut [u8]) -> IoResult<usize>;
    async fn write(&self, offset: usize, buf: [u8]) -> IoResult<usize>;
    async fn write_zero(&self, offset: usize, length: usize) -> IoResult<usize>;
    async fn flush(&self) -> IoResult<()>;
    async fn trim(&self, offset: usize, length: usize) -> IoResult<()>;
    fn dup(&self) -> Box<dyn Image>;
}

pub struct ImageStat {
    size: usize,
}

pub trait ImageProvider {}
