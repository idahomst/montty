pub mod bindings;
pub mod keys;
pub mod mouse;
pub mod paste;

use winit::keyboard::ModifiersState;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub logo: bool,
}

impl From<ModifiersState> for Mods {
    fn from(m: ModifiersState) -> Self {
        Mods {
            ctrl: m.control_key(),
            alt: m.alt_key(),
            shift: m.shift_key(),
            logo: m.super_key(),
        }
    }
}
