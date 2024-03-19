#![allow(unused_variables)]

use std::{
    io::IoSlice,
    ops::{Bound, Deref, DerefMut, RangeBounds},
};

use bytes::{Buf, Bytes, BytesMut};
pub use smallvec::SmallVec;

const BYTES_VEC_INLINE_SIZE: usize = 1;

#[derive(Clone)]
pub struct BytesVec {
    bytes_vec: SmallVec<[Bytes; BYTES_VEC_INLINE_SIZE]>,
}

impl BytesVec {
    pub fn new() -> Self {
        BytesVec {
            bytes_vec: SmallVec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.bytes_vec
            .iter()
            .fold(0, |last, item| last + item.len())
    }

    pub fn vec_count(&self) -> usize {
        self.bytes_vec.len()
    }

    pub fn merge(&mut self) {
        let mut bytes_mut = BytesMut::with_capacity(self.len());
        bytes_mut.extend(self.drain(..));
        let bytes = bytes_mut.freeze();
        self.bytes_vec.clear();
        self.bytes_vec.push(bytes);
    }

    pub fn clear(&mut self) {
        self.bytes_vec.clear();
    }

    pub fn slice(&self, range: impl RangeBounds<usize>) -> Self {
        let len = self.len();
        let begin = match range.start_bound() {
            Bound::Included(&n) => n,
            Bound::Excluded(&n) => n + 1,
            Bound::Unbounded => 0,
        };
        let end = match range.end_bound() {
            Bound::Included(&n) => n + 1,
            Bound::Excluded(&n) => n,
            Bound::Unbounded => len,
        };

        assert!(begin <= end);
        assert!(end <= len);

        if begin == end {
            return BytesVec::new();
        }

        let mut new_vec = BytesVec::new();
        let mut cur_len = 0;
        for item in &self.bytes_vec {
            if cur_len + item.len() <= begin || cur_len >= end {
                cur_len += item.len();
                continue;
            }
            // This item is reside in the range.
            if cur_len < begin {
                new_vec.bytes_vec.push(item.slice((begin - cur_len)..));
            } else if cur_len + item.len() > end {
                new_vec.bytes_vec.push(item.slice(..(end - cur_len)));
            } else {
                new_vec.bytes_vec.push(item.clone());
            }
            cur_len += item.len();
        }
        new_vec
    }

    pub fn split_off(&mut self, at: usize) -> Self {
        let mut new_vec = BytesVec::new();
        self.split_off_impl(at, Some(&mut new_vec));
        new_vec
    }

    fn split_off_impl(&mut self, at: usize, mut remain: Option<&mut BytesVec>) {
        assert!(at <= self.len());

        // let mut new_vec = BytesVec::new();
        let mut cur_len = 0;
        let mut cur_pos = 0;
        for item in &mut self.bytes_vec {
            if cur_len <= at && cur_len + item.len() > at {
                // Split here.
                let new_item = item.split_off(at - cur_len);
                if let Some(remain) = &mut remain {
                    remain.bytes_vec.push(new_item);
                }

                if cur_pos + 1 < self.bytes_vec.len() {
                    // Remove all other elements.
                    self.bytes_vec.drain((cur_pos + 1)..).for_each(|item| {
                        if let Some(remain) = &mut remain {
                            remain.bytes_vec.push(item)
                        }
                    });
                }
                break;
            }

            cur_len += item.len();
            cur_pos += 1;
        }
    }

    pub fn split_to(&mut self, at: usize) -> Self {
        let mut new_vec = BytesVec::new();
        self.split_to_impl(at, Some(&mut new_vec));
        new_vec
    }

    fn split_to_impl(&mut self, at: usize, mut remain: Option<&mut BytesVec>) {
        assert!(at <= self.len());

        let mut cur_len = 0;

        while !self.bytes_vec.is_empty() {
            let item = &mut self.bytes_vec[0];
            if cur_len <= at && cur_len + item.len() > at {
                // Split here
                let new_item = item.split_to(at - cur_len);
                if let Some(remain) = &mut remain {
                    remain.bytes_vec.push(new_item);
                }
                break;
            }
            cur_len += item.len();
            if let Some(remain) = &mut remain {
                remain.bytes_vec.push(self.bytes_vec.remove(0));
            }
        }
    }

    pub fn truncate(&mut self, len: usize) {
        assert!(len <= self.len());
        let mut cur_len = 0;
        let mut cur_pos = 0;
        for item in &mut self.bytes_vec {
            if cur_len + item.len() > len {
                let item_len = len - cur_len;
                item.truncate(item_len);
                self.bytes_vec.truncate(cur_pos + 1);
                break;
            }
            cur_len += item.len();
            cur_pos += 1;
        }
    }

    pub fn io_slice(&self) -> Vec<IoSlice<'_>> {
        self.bytes_vec
            .iter()
            .map(|item| IoSlice::new(&item))
            .collect()
    }

    pub fn push<T>(&mut self, data: T)
    where
        T: AsRef<[u8]>,
    {
        let bytes = Bytes::copy_from_slice(data.as_ref());
        self.bytes_vec.push(bytes);
    }

    pub fn push_static(&mut self, data: &'static [u8]) {
        let bytes = Bytes::from_static(data);
        self.bytes_vec.push(bytes);
    }

    pub fn push_bytes(&mut self, data: Bytes) {
        self.bytes_vec.push(data);
    }

    pub fn buf_iter(&self) -> BufIter<'_> {
        BufIter {
            bytes_vec: self,
            cur_item: 0,
        }
    }

    pub fn data_iter(&self) -> DataIter<'_> {
        let mut buf_iter = self.buf_iter();
        let cur_buf = buf_iter.next();
        DataIter {
            buf_iter,
            cur_buf,
            cur_pos: 0,
        }
    }

    pub fn into_vec(&self) -> Vec<u8> {
        self.into()
    }
}

pub struct BufIter<'a> {
    bytes_vec: &'a BytesVec,
    cur_item: usize,
}

impl<'a> Iterator for BufIter<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        if self.cur_item >= self.bytes_vec.vec_count() {
            None
        } else {
            let res = self.bytes_vec[self.cur_item].deref();
            self.cur_item += 1;
            Some(res)
        }
    }
}

pub struct DataIter<'a> {
    buf_iter: BufIter<'a>,
    cur_buf: Option<&'a [u8]>,
    cur_pos: usize,
}

impl<'a> Iterator for DataIter<'a> {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.cur_buf.is_none() {
                return None;
            }

            let cur_buf = self.cur_buf.unwrap();
            if self.cur_pos < cur_buf.len() {
                let res = cur_buf[self.cur_pos];
                self.cur_pos += 1;
                return Some(res);
            }
            self.cur_pos = 0;
            self.cur_buf = self.buf_iter.next();
        }
    }
}

impl Deref for BytesVec {
    type Target = SmallVec<[Bytes; BYTES_VEC_INLINE_SIZE]>;

    fn deref(&self) -> &Self::Target {
        &self.bytes_vec
    }
}

impl DerefMut for BytesVec {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.bytes_vec
    }
}

impl From<BytesVec> for Vec<u8> {
    fn from(value: BytesVec) -> Self {
        let mut res = Vec::with_capacity(value.len());
        value.buf_iter().for_each(|buf| res.extend(buf));
        res
    }
}

impl From<&BytesVec> for Vec<u8> {
    fn from(value: &BytesVec) -> Self {
        let mut res = Vec::with_capacity(value.len());
        value.buf_iter().for_each(|buf| res.extend(buf));
        res
    }
}

impl Buf for BytesVec {
    fn advance(&mut self, cnt: usize) {
        self.split_to(cnt);
    }

    fn chunk(&self) -> &[u8] {
        if self.vec_count() == 0 {
            &[]
        } else {
            &self.bytes_vec[0]
        }
    }

    fn remaining(&self) -> usize {
        self.len()
    }
}

#[cfg(test)]
mod test {

    use super::*;
    const STR: &[u8] = b"hello world!";

    fn create_bytes_vec() -> BytesVec {
        let mut bytes_vec = BytesVec::new();
        bytes_vec.push("hello ");
        bytes_vec.push_static(b"world");
        bytes_vec.push_bytes(Bytes::copy_from_slice(b"!"));
        bytes_vec
    }

    #[test]
    fn test_bytes_vec() {
        let mut bytes_vec = create_bytes_vec();

        assert_eq!(bytes_vec.vec_count(), 3);
        assert_eq!(bytes_vec.len(), STR.len());

        let mut buf_iter = bytes_vec.buf_iter();
        assert_eq!(buf_iter.next(), Some(b"hello ".as_slice()));
        assert_eq!(buf_iter.next(), Some(b"world".as_slice()));
        assert_eq!(buf_iter.next(), Some(b"!".as_slice()));
        assert_eq!(buf_iter.next(), None);

        let buf: Vec<_> = bytes_vec.data_iter().collect();
        assert_eq!(buf.as_slice(), STR);

        bytes_vec.merge();
        assert_eq!(bytes_vec.vec_count(), 1);
        assert_eq!(bytes_vec.len(), STR.len());
        let mut buf_iter = bytes_vec.buf_iter();
        assert_eq!(buf_iter.next(), Some(STR));
        assert_eq!(buf_iter.next(), None);

        bytes_vec.clear();
        assert_eq!(bytes_vec.vec_count(), 0);
        assert_eq!(bytes_vec.len(), 0);
    }

    #[test]
    fn test_bytes_vec_slice() {
        let mut bytes_vec = create_bytes_vec();

        assert_eq!(bytes_vec.slice(6..).into_vec(), Vec::from("world!"));
        assert_eq!(bytes_vec.slice(..5).into_vec(), Vec::from("hello"));
        assert_eq!(bytes_vec.slice(1..9).into_vec(), Vec::from("ello wor"));

        bytes_vec.truncate(9);
        assert_eq!(bytes_vec.into_vec(), Vec::from("hello wor"));
    }

    #[test]
    fn test_bytes_vec_split_off() {
        let mut bytes_vec = create_bytes_vec();
        let other = bytes_vec.split_off(8);

        assert_eq!(bytes_vec.into_vec(), Vec::from("hello wo"));
        assert_eq!(other.into_vec(), Vec::from("rld!"));
    }

    #[test]
    fn test_bytes_vec_split_to() {
        let mut bytes_vec = create_bytes_vec();
        let other = bytes_vec.split_to(8);

        assert_eq!(bytes_vec.into_vec(), Vec::from("rld!"));
        assert_eq!(other.into_vec(), Vec::from("hello wo"));
    }
}
