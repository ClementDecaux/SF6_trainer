//! Les touches du clavier lues même quand SF6 a le focus :
//! F8 active ou coupe le déplacement de l'overlay, F9 l'enregistrement des vidéos.

pub struct Touche {
    /// Code Windows de la touche (VK_F8...).
    code: u16,
    avant: bool,
}

impl Touche {
    pub fn f8() -> Touche {
        Touche {
            code: 0x77,
            avant: false,
        }
    }

    pub fn f9() -> Touche {
        Touche {
            code: 0x78,
            avant: false,
        }
    }

    /// Vrai une seule fois, au moment où la touche est enfoncée.
    pub fn appuyee(&mut self) -> bool {
        let enfoncee = enfoncee(self.code);
        let nouvelle = enfoncee && !self.avant;
        self.avant = enfoncee;
        nouvelle
    }
}

#[cfg(windows)]
fn enfoncee(code: u16) -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    // Le bit de poids fort dit si la touche est enfoncée en ce moment, quelle que soit
    // la fenêtre qui a le focus.
    unsafe { (GetAsyncKeyState(code as i32) as u16) & 0x8000 != 0 }
}

#[cfg(not(windows))]
fn enfoncee(_code: u16) -> bool {
    false
}
