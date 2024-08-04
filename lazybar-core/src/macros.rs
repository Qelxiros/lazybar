/// Defines a struct to hold format strings, along with a constructor.
///
/// The constructor has the following function signature:
/// ```rust
/// fn new(value: Vec<T>) -> Self
/// ```
/// `value` must have the same number of elements as `args` passed to this
/// macro, and `new` will panic otherwise.
#[macro_export]
macro_rules! array_to_struct {
    ($name:ident, $($args:ident),+) => {
        #[derive(Debug, Default, Clone, PartialEq, Eq, PartialOrd, Ord)]
        struct $name<T> {
            $(
                $args: T,
            )+
        }

        impl<T> $name<T> {
            fn new<const N: usize>(value: [T; N]) -> Self {
                let mut value = value.into_iter();

                Self {
                    $(
                        $args: value.next().unwrap(),
                    )+
                }
            }
        }
    };
}

/// Holds a collection of X atoms, lazily checking their values as they're
/// retrieved.
///
/// The first argument defines the struct name, the second argument should be a
/// static reference to a struct of this type, and each argument after that is
/// the name of an atom. The struct has a constructor with signature:
/// ```rust
/// const fn new() -> Self
/// ```
/// and a method with signature:
/// ```rust
/// pub fn get(&mut self, conn: &impl Connection, atom_name: &'static str) -> Result<u32>
/// ```
/// This macro is for internal use and should be called elsewhere with care.
#[macro_export]
macro_rules! interned_atoms {
    ($name:ident, $ref:expr, $($atoms:ident,)+) => {
        #[allow(non_snake_case)]
        pub struct $name {
            $(
                $atoms: ::std::primitive::u32,
            )+
        }

        impl $name {
            pub const fn new() -> Self {
                unsafe { ::std::mem::zeroed() }
            }

            fn get_inner(
                &mut self,
                conn: &impl ::x11rb::connection::Connection,
                atom_name: &'static ::std::primitive::str,
            ) -> Result<::std::primitive::u32> {
                let atom = match atom_name {
                    $(
                        stringify!($atoms) => ::std::option::Option::Some(self.$atoms),
                    )+
                    _ => ::std::option::Option::None,
                };

                match atom {
                    ::std::option::Option::None => ::std::result::Result::Err(::anyhow::anyhow!("Invalid atom name")),
                    ::std::option::Option::Some(0) => {
                        let atom =
                            $crate::x::intern_named_atom(conn, atom_name.as_bytes())?;
                        match atom_name {
                            $(
                                ::std::stringify!($atoms) => self.$atoms = atom,
                            )+
                            _ => ::std::unreachable!(),
                        };
                        ::std::result::Result::Ok(atom)
                    }
                    ::std::option::Option::Some(atom) => ::std::result::Result::Ok(atom),
                }
            }

            pub fn get(conn: &impl Connection, atom_name: &'static str) -> Result<u32> {
                $name::get_inner(&mut *$ref.lock().unwrap(), conn, atom_name)
            }
        }
    };
}
