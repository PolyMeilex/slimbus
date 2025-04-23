use zvariant::Str;

pub type InterfaceName<'a> = Str<'a>;
pub type MemberName<'a> = Str<'a>;
pub type UniqueName<'a> = Str<'a>;
pub type ErrorName<'a> = Str<'a>;
pub type BusName<'a> = Str<'a>;

pub type OwnedErrorName = ErrorName<'static>;
pub type OwnedUniqueName = UniqueName<'static>;
