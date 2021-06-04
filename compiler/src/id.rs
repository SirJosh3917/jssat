macro_rules! gen_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name(::std::num::NonZeroUsize);

        impl $name {
            pub fn new() -> Self {
                <Self as crate::id::IdCompat>::new_with_value(0)
            }

            pub fn next(&self) -> Self {
                <Self as crate::id::IdCompat>::new_with_value(
                    <Self as crate::id::IdCompat>::value(&self) + 1,
                )
            }
        }

        impl crate::id::IdCompat for $name {
            fn new_with_value(value: usize) -> Self {
                debug_assert!(value != usize::MAX);
                Self(::std::num::NonZeroUsize::new(value + 1).unwrap())
            }

            fn value(&self) -> usize {
                self.0.get() - 1
            }
        }
    };
}

pub trait IdCompat {
    fn new_with_value(value: usize) -> Self;

    fn value(&self) -> usize;
}

pub fn convert<A, B>(a: A) -> B
where
    A: IdCompat,
    B: IdCompat,
{
    B::new_with_value(a.value())
}

gen_id!(TypeId);
gen_id!(TopLevelId);
gen_id!(BlockId);
gen_id!(RegisterId);
gen_id!(InternalSlotId);