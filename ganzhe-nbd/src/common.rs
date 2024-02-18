use paste::paste;

macro_rules! define_endian_impl {
    ($base:ident, $endian:ident) => {
        paste! {
            #[repr(transparent)]
            #[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Hash)]
            pub struct [< $endian:camel $base:camel >]($base);

            impl [< $endian:camel $base:camel >] {
                pub fn new(value: $base) -> Self {
                    value.into()
                }

                pub fn inner(&self) -> &$base {
                    &self.0
                }

                pub fn inner_mut(&mut self) -> &mut $base {
                    &mut self.0
                }
            }

            impl From<[< $endian:camel $base:camel >]> for $base {
                fn from(value: [< $endian:camel $base:camel >]) -> Self {
                    $base::[< from_ $endian >](value.0)
                }
            }

            impl From<$base> for [< $endian:camel $base:camel >] {
                fn from(value: $base) -> Self {
                    Self($base::[< to_ $endian >](value))
                }
            }

        }
    };
}

macro_rules! define_endian {
    ($base:ident) => {
        define_endian_impl!($base, be);
        define_endian_impl!($base, le);
        paste! {
            pub type [< Ne $base:camel >] = [< Be $base:camel >];
        }
    };
}

define_endian!(u8);
define_endian!(i8);
define_endian!(u16);
define_endian!(i16);
define_endian!(u32);
define_endian!(i32);
define_endian!(u64);
define_endian!(i64);
define_endian!(usize);
define_endian!(isize);

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_endian() {
        let v = BeU16::new(10);
        assert_eq!(*v.inner(), 10u16.to_be());
        assert_eq!(u16::from(v), 10u16);
        let x = NeU16::new(10);
        assert_eq!(v, x);
    }
}
