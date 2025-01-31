use std::{
    cmp::Ordering,
    fmt,
    hash::{Hash, Hasher},
    ops::Deref,
    rc::Rc,
};

use super::Address;

/// An immutable pointer to a value in allocated memory
#[derive(Debug, Default)]
pub struct Ptr<T: ?Sized>(Rc<T>);

impl<T> Ptr<T> {
    /// Moves the value into newly allocated memory
    pub fn new(value: T) -> Self {
        Self(Rc::new(value))
    }
}

impl<T: ?Sized> Ptr<T> {
    /// Returns true if the two `Ptr`s point to the same allocation
    ///
    /// See also: [std::rc::Rc::ptr_eq]
    pub fn ptr_eq(this: &Self, other: &Self) -> bool {
        Rc::ptr_eq(&this.0, &other.0)
    }

    /// Returns the address of the allocated memory
    pub fn address(this: &Self) -> Address {
        Rc::as_ptr(&this.0).into()
    }
}

impl<T: Clone> Ptr<T> {
    /// Makes a mutable reference into the owned `T`
    ///
    /// If the pointer has the only reference to the value, then the reference will be returned.
    /// Otherwise a clone of the value will be made to ensure uniqueness before returning the
    /// reference.
    ///
    /// See also: [std::rc::Rc::make_mut]
    pub fn make_mut(this: &mut Self) -> &mut T {
        Rc::make_mut(&mut this.0)
    }
}

impl<T> From<T> for Ptr<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl<T: ?Sized> From<Box<T>> for Ptr<T> {
    fn from(boxed: Box<T>) -> Self {
        Self(boxed.into())
    }
}

impl<T: ?Sized> From<Rc<T>> for Ptr<T> {
    fn from(inner: Rc<T>) -> Self {
        Self(inner)
    }
}

impl<T: ?Sized> Deref for Ptr<T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.0.deref()
    }
}

impl<T: ?Sized> Clone for Ptr<T> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<T: Clone> From<&[T]> for Ptr<[T]> {
    #[inline]
    fn from(value: &[T]) -> Self {
        Self(Rc::from(value))
    }
}

impl<T> From<Vec<T>> for Ptr<[T]> {
    #[inline]
    fn from(value: Vec<T>) -> Self {
        Self(Rc::from(value))
    }
}

impl From<&str> for Ptr<str> {
    #[inline]
    fn from(value: &str) -> Self {
        Self(Rc::from(value))
    }
}

impl From<String> for Ptr<str> {
    #[inline]
    fn from(value: String) -> Self {
        Self(Rc::from(value))
    }
}

impl<T: ?Sized + PartialEq> PartialEq for Ptr<T> {
    fn eq(&self, other: &Self) -> bool {
        Rc::eq(&self.0, &other.0)
    }
}

impl<T: ?Sized + Eq> Eq for Ptr<T> {}

impl<T: ?Sized + fmt::Display> fmt::Display for Ptr<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<T: ?Sized + Hash> Hash for Ptr<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state)
    }
}

impl<T: ?Sized + Ord> Ord for Ptr<T> {
    #[inline]
    fn cmp(&self, other: &Ptr<T>) -> Ordering {
        self.0.cmp(&other.0)
    }
}

impl<T: ?Sized + PartialOrd> PartialOrd for Ptr<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.0.partial_cmp(&other.0)
    }
}
