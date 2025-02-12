//! Array with SIMD alignment

use std::convert::TryInto;
use std::ops::{Deref, DerefMut};
use std::os::raw::c_void;
use std::slice::{from_raw_parts, from_raw_parts_mut};

use num_traits::Zero;

use crate::types::*;

/// A RAII-wrapper of `fftw_alloc` and `fftw_free` with the [SIMD alignment].
///
/// [SIMD alignment]: http://www.fftw.org/fftw3_doc/SIMD-alignment-and-fftw_005fmalloc.html
#[derive(Debug)]
pub struct AlignedVec<T> {
    n: usize,
    data: *mut T,
}

/// Allocate SIMD-aligned memory of Real/Complex type
pub trait AlignedAllocable: Zero + Clone + Copy + Sized {
    /// Allocate SIMD-aligned memory
    #[allow(clippy::missing_safety_doc)]
    unsafe fn alloc(n: usize) -> *mut Self;
}

impl AlignedAllocable for f64 {
    unsafe fn alloc(n: usize) -> *mut Self {
        ffi::fftw_alloc_real(n.try_into().unwrap())
    }
}

impl AlignedAllocable for f32 {
    unsafe fn alloc(n: usize) -> *mut Self {
        ffi::fftwf_alloc_real(n.try_into().unwrap())
    }
}

impl AlignedAllocable for c64 {
    unsafe fn alloc(n: usize) -> *mut Self {
        ffi::fftw_alloc_complex(n.try_into().unwrap()) as *mut _
    }
}

impl AlignedAllocable for c32 {
    unsafe fn alloc(n: usize) -> *mut Self {
        ffi::fftwf_alloc_complex(n.try_into().unwrap()) as *mut c32
    }
}

impl<T> AlignedVec<T> {
    pub fn as_slice(&self) -> &[T] {
        unsafe { from_raw_parts(self.data, self.n) }
    }

    pub fn as_slice_mut(&mut self) -> &mut [T] {
        unsafe { from_raw_parts_mut(self.data, self.n) }
    }
}

impl<T> Deref for AlignedVec<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        self.as_slice()
    }
}

impl<T> DerefMut for AlignedVec<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        self.as_slice_mut()
    }
}

impl<T> AlignedVec<T>
where
    T: AlignedAllocable,
{
    /// Create array with `fftw_malloc` (`fftw_free` will be automatically called by `Drop` trait)
    pub fn new(n: usize) -> Self {
        let ptr = excall! { T::alloc(n) };
        let mut vec = AlignedVec { n, data: ptr };
        for v in vec.iter_mut() {
            *v = T::zero();
        }
        vec
    }
}

impl<T> Drop for AlignedVec<T> {
    fn drop(&mut self) {
        excall! { ffi::fftw_free(self.data as *mut c_void) };
    }
}

impl<T> Clone for AlignedVec<T>
where
    T: AlignedAllocable,
{
    fn clone(&self) -> Self {
        let mut new_vec = Self::new(self.n);
        new_vec.copy_from_slice(self);
        new_vec
    }
}

impl<T> PartialEq for AlignedVec<T>
where
    T: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        if self.len() != other.len() {
            return false;
        }
        self.iter().zip(other.iter()).all(|(a, b)| a == b)
    }
}

unsafe impl<T: Send> Send for AlignedVec<T> {}
unsafe impl<T: Sync> Sync for AlignedVec<T> {}

pub type Alignment = i32;

/// Check the alignment of slice
///
/// ```
/// # use concrete_fftw::array::*;
/// let a = AlignedVec::<f32>::new(123);
/// assert_eq!(alignment_of(&a), 0);  // aligned
/// ```
pub fn alignment_of<T>(a: &[T]) -> Alignment {
    unsafe { ffi::fftw_alignment_of(a.as_ptr() as *mut _) }
}

#[cfg(feature = "serialize")]
mod serde {
    use std::fmt;
    use std::marker::PhantomData;

    use serde::de::{Error, SeqAccess, Visitor};
    use serde::ser::{Serialize, SerializeSeq, Serializer};
    use serde::{Deserialize, Deserializer};

    use crate::array::AlignedAllocable;

    use super::AlignedVec;

    impl<T> Serialize for AlignedVec<T>
    where
        T: Serialize,
    {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let mut seq = serializer.serialize_seq(Some(self.len()))?;
            for e in self.iter() {
                seq.serialize_element(e)?;
            }
            seq.end()
        }
    }

    struct AlignedVecVisitor<T>(PhantomData<T>);

    impl<'de, T> Visitor<'de> for AlignedVecVisitor<T>
    where
        T: AlignedAllocable + Deserialize<'de>,
    {
        type Value = AlignedVec<T>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            write!(formatter, "AlignedVec<T>")
        }

        fn visit_seq<A>(self, seq: A) -> Result<Self::Value, <A as SeqAccess<'de>>::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut seq = seq;
            let mut output = AlignedVec::new(seq.size_hint().ok_or(A::Error::custom(
                "Failed to retrieve the size of the AlignedVec.",
            ))?);
            for val in output.iter_mut() {
                *val = seq
                    .next_element()?
                    .ok_or(A::Error::custom("Failed to retrieve the next element"))?
            }
            Ok(output)
        }
    }

    impl<'de, T> Deserialize<'de> for AlignedVec<T>
    where
        T: AlignedAllocable + Deserialize<'de>,
    {
        fn deserialize<D>(deserializer: D) -> Result<Self, <D as Deserializer<'de>>::Error>
        where
            D: Deserializer<'de>,
        {
            deserializer.deserialize_seq(AlignedVecVisitor(PhantomData))
        }
    }

    #[cfg(test)]
    mod test {
        use serde_test::{assert_tokens, Token};

        use crate::types::{c32, c64};

        use super::AlignedVec;

        #[test]
        fn test_ser_de_empty_c32() {
            let vec: AlignedVec<c32> = AlignedVec::new(0);

            assert_tokens(&vec, &[Token::Seq { len: Some(0) }, Token::SeqEnd]);
        }

        #[test]
        fn test_ser_de_empty_c64() {
            let vec: AlignedVec<c64> = AlignedVec::new(0);

            assert_tokens(&vec, &[Token::Seq { len: Some(0) }, Token::SeqEnd]);
        }

        #[test]
        fn test_ser_de_c32() {
            let mut vec = AlignedVec::new(3);
            vec[0] = c32::new(1., 2.);
            vec[1] = c32::new(3., 4.);
            vec[2] = c32::new(5., 6.);

            assert_tokens(
                &vec,
                &[
                    Token::Seq { len: Some(3) },
                    Token::Tuple { len: 2 },
                    Token::F32(1.),
                    Token::F32(2.),
                    Token::TupleEnd,
                    Token::Tuple { len: 2 },
                    Token::F32(3.),
                    Token::F32(4.),
                    Token::TupleEnd,
                    Token::Tuple { len: 2 },
                    Token::F32(5.),
                    Token::F32(6.),
                    Token::TupleEnd,
                    Token::SeqEnd,
                ],
            );
        }

        #[test]
        fn test_ser_de_c64() {
            let mut vec = AlignedVec::new(3);
            vec[0] = c64::new(1., 2.);
            vec[1] = c64::new(3., 4.);
            vec[2] = c64::new(5., 6.);

            assert_tokens(
                &vec,
                &[
                    Token::Seq { len: Some(3) },
                    Token::Tuple { len: 2 },
                    Token::F64(1.),
                    Token::F64(2.),
                    Token::TupleEnd,
                    Token::Tuple { len: 2 },
                    Token::F64(3.),
                    Token::F64(4.),
                    Token::TupleEnd,
                    Token::Tuple { len: 2 },
                    Token::F64(5.),
                    Token::F64(6.),
                    Token::TupleEnd,
                    Token::SeqEnd,
                ],
            );
        }
    }
}
