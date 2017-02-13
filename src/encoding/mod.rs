mod never;
mod pointer;
mod primitive;
mod structure;

use std::fmt;

use multi::EncodingsComparator;

pub use self::never::Never;
pub use self::pointer::Pointer;
pub use self::primitive::Primitive;
pub use self::structure::Struct;

pub trait Encoding: fmt::Display {
    type Pointer: ?Sized + PointerEncoding;
    type Struct: ?Sized + StructEncoding;

    fn descriptor(&self) -> Descriptor<Self::Pointer, Self::Struct>;

    fn eq_encoding<T: ?Sized + Encoding>(&self, &T) -> bool;
}

pub trait StructEncoding: Encoding {
    fn name(&self) -> &str;
    fn eq_struct<T: EncodingsComparator>(&self, name: &str, fields: T) -> bool;
}

pub trait PointerEncoding: Encoding {
    type Pointee: ?Sized + Encoding;

    fn pointee(&self) -> &Self::Pointee;
}

pub enum Descriptor<'a, P, S>
        where P: 'a + ?Sized + PointerEncoding,
              S: 'a + ?Sized + StructEncoding {
    Primitive(Primitive),
    Pointer(&'a P),
    Struct(&'a S),
}

impl<'a, P, S> Descriptor<'a, P, S>
        where P: 'a + ?Sized + PointerEncoding,
              S: 'a + ?Sized + StructEncoding {
    pub fn eq_encoding<T: ?Sized + Encoding>(&self, other: &T) -> bool {
        match *self {
            Descriptor::Primitive(p) => p.eq_encoding(other),
            Descriptor::Pointer(p) => p.eq_encoding(other),
            Descriptor::Struct(s) => s.eq_encoding(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parse::StrEncoding;

    #[test]
    fn test_int_display() {
        assert_eq!(Primitive::Int.to_string(), "i");
    }

    #[test]
    fn test_pointer_display() {
        let e = Pointer::new(Primitive::Int);
        assert_eq!(e.to_string(), "^i");
    }

    #[test]
    fn test_static_struct() {
        let f = (Primitive::Char, Primitive::Int);
        let s = Struct::new("CGPoint", f);
        assert_eq!(s.name(), "CGPoint");
        assert_eq!(s.to_string(), "{CGPoint=ci}");
    }

    #[test]
    fn test_eq_encoding() {
        let i = Primitive::Int;
        let c = Primitive::Char;

        assert!(i.eq_encoding(&i));
        assert!(!i.eq_encoding(&c));

        let p = Pointer::new(i);
        assert!(p.eq_encoding(&p));
        assert!(!p.eq_encoding(&i));

        let s = Struct::new("CGPoint", (c, i));
        assert!(s.eq_encoding(&s));
        assert!(!s.eq_encoding(&i));

        let s2 = StrEncoding::new_unchecked("{CGPoint=ci}");
        assert!(s2.eq_encoding(&s2));
        assert!(s.eq_encoding(&s2));
    }
}
