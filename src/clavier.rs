//! La touche F8, lue même quand SF6 a le focus : elle active ou coupe le mode
//! déplacement de l'overlay (pendant ce mode, on peut le faire glisser à la souris).

pub struct ToucheDeplacement {
    avant: bool,
}

impl ToucheDeplacement {
    pub fn new() -> ToucheDeplacement {
        ToucheDeplacement { avant: false }
    }

    /// Vrai une seule fois, au moment où F8 est enfoncée.
    pub fn appuyee(&mut self) -> bool {
        let enfoncee = f8_enfoncee();
        let nouvelle = enfoncee && !self.avant;
        self.avant = enfoncee;
        nouvelle
    }
}

#[cfg(windows)]
fn f8_enfoncee() -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_F8};
    // Le bit de poids fort dit si la touche est enfoncée en ce moment, quelle que soit
    // la fenêtre qui a le focus.
    unsafe { (GetAsyncKeyState(VK_F8 as i32) as u16) & 0x8000 != 0 }
}

#[cfg(not(windows))]
fn f8_enfoncee() -> bool {
    false
}
