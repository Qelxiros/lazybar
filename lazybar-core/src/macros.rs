/// Defines a struct to hold format strings, along with a constructor.
///
/// The constructor has the following function signature:
/// ```rust
/// fn new(value: Vec<String>) -> Self
/// ```
/// `value` must have the same number of elements as `args` passed to this
/// macro, and `new` will panic otherwise.
#[macro_export]
macro_rules! format_struct {
    ($name:ident, $($args:ident),+) => {
        #[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
        struct $name {
            $(
                $args: &'static str,
            )+
        }

        impl $name {
            fn new(value: ::std::vec::Vec<::std::string::String>) -> Self {
                let mut value = ::std::iter::Iterator::map(::std::iter::IntoIterator::into_iter(value), |s| ::std::string::String::leak(s));

                Self {
                    $(
                        $args: ::std::iter::Iterator::next(&mut value).unwrap(),
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
                $atoms: u32,
            )+
        }

        impl $name {
            pub const fn new() -> Self {
                unsafe { mem::zeroed() }
            }

            fn get_inner(
                &mut self,
                conn: &impl Connection,
                atom_name: &'static str,
            ) -> Result<u32> {
                let atom = match atom_name {
                    $(
                        stringify!($atoms) => Some(self.$atoms),
                    )+
                    _ => None,
                };

                match atom {
                    None => Err(anyhow!("Invalid atom name")),
                    Some(0) => {
                        let atom =
                            conn.intern_atom(true, atom_name.as_bytes())?.reply()?.atom;
                        match atom_name {
                            $(
                                stringify!($atoms) => self.$atoms = atom,
                            )+
                            _ => unreachable!(),
                        };
                        Ok(atom)
                    }
                    Some(atom) => Ok(atom),
                }
            }

            pub fn get(conn: &impl Connection, atom_name: &'static str) -> Result<u32> {
                unsafe { $ref.get_inner(conn, atom_name) }
            }
        }
    };
}
