//! Lecture de la manette.
//!
//! Sous Windows, on interroge XInput nous-mêmes à chaque tour de boucle (toutes les
//! millisecondes). La bibliothèque gilrs ne le fait que toutes les 10 à 15 ms : presque
//! une frame d'imprécision, assez pour décaler nos durées d'une frame par rapport au jeu.
//! Ailleurs (Linux), on garde gilrs.
//!
//! Les boutons gardent les noms de gilrs (`South`, `RightTrigger2`...) : `controles.toml`
//! reste le même.

// Hors Windows, pas de capture ni de XInput : une partie du code n'y sert pas.
#![cfg_attr(not(windows), allow(dead_code))]

/// Ce qui vient de changer sur la manette.
#[derive(Debug, Clone, PartialEq)]
pub enum Evenement {
    Appui(&'static str),
    Relache(&'static str),
    /// Stick gauche, de -1 (gauche / bas) à 1 (droite / haut).
    StickX(f32),
    StickY(f32),
    Connectee(String),
    Deconnectee,
}

/// Une photo de l'état d'une manette XInput.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Etat {
    pub boutons: u16,
    pub gachette_gauche: u8,
    pub gachette_droite: u8,
    pub stick_x: i16,
    pub stick_y: i16,
}

/// Les bits des boutons XInput, avec le nom que leur donne gilrs.
const BOUTONS: [(u16, &str); 14] = [
    (0x0001, "DPadUp"),
    (0x0002, "DPadDown"),
    (0x0004, "DPadLeft"),
    (0x0008, "DPadRight"),
    (0x0010, "Start"),
    (0x0020, "Select"),
    (0x0040, "LeftThumb"),
    (0x0080, "RightThumb"),
    (0x0100, "LeftTrigger"),
    (0x0200, "RightTrigger"),
    (0x1000, "South"),
    (0x2000, "East"),
    (0x4000, "West"),
    (0x8000, "North"),
];

/// Une gâchette compte comme appuyée au-dessus de la moitié de sa course.
const GACHETTE_APPUI: u8 = 128;
/// ...et relâchée en dessous de 100 (un écart évite les appuis fantômes à la limite).
const GACHETTE_RELACHE: u8 = 100;

/// Tout ce qui a changé entre deux photos de la manette.
pub fn differences(avant: &Etat, apres: &Etat) -> Vec<Evenement> {
    let mut evenements = Vec::new();
    for &(bit, nom) in &BOUTONS {
        let a = avant.boutons & bit != 0;
        let b = apres.boutons & bit != 0;
        if !a && b {
            evenements.push(Evenement::Appui(nom));
        } else if a && !b {
            evenements.push(Evenement::Relache(nom));
        }
    }
    for (a, b, nom) in [
        (avant.gachette_gauche, apres.gachette_gauche, "LeftTrigger2"),
        (
            avant.gachette_droite,
            apres.gachette_droite,
            "RightTrigger2",
        ),
    ] {
        let etait = a >= GACHETTE_APPUI;
        let maintenant = if etait {
            b >= GACHETTE_RELACHE
        } else {
            b >= GACHETTE_APPUI
        };
        if maintenant && !etait {
            evenements.push(Evenement::Appui(nom));
        } else if etait && !maintenant {
            evenements.push(Evenement::Relache(nom));
        }
    }
    if avant.stick_x != apres.stick_x {
        evenements.push(Evenement::StickX(apres.stick_x as f32 / 32767.0));
    }
    if avant.stick_y != apres.stick_y {
        evenements.push(Evenement::StickY(apres.stick_y as f32 / 32767.0));
    }
    evenements
}

#[cfg(windows)]
pub use windows::Manette;

#[cfg(windows)]
mod windows {
    use super::{Etat, Evenement, differences};
    use windows_sys::Win32::Media::timeBeginPeriod;
    use windows_sys::Win32::UI::Input::XboxController::{XINPUT_STATE, XInputGetState};

    /// Une manette absente met du temps à répondre : on ne cherche les nouvelles
    /// manettes qu'une fois toutes les 1000 lectures (environ une seconde).
    const RECHERCHE_TOUS_LES: u32 = 1000;

    pub struct Manette {
        /// Pour chaque emplacement XInput (0 à 3) : son dernier état, s'il est branché.
        etats: [Option<(u32, Etat)>; 4],
        tours: u32,
    }

    impl Manette {
        pub fn new() -> Result<Manette, String> {
            // Des pauses d'une vraie milliseconde (sinon Windows arrondit à ~15 ms).
            unsafe {
                timeBeginPeriod(1);
            }
            Ok(Manette {
                etats: [None; 4],
                tours: 0,
            })
        }

        pub fn noms(&self) -> Vec<String> {
            Vec::new()
        }

        /// Interroge les manettes et renvoie ce qui a changé depuis la dernière fois.
        pub fn lire(&mut self) -> Vec<Evenement> {
            let chercher = self.tours.is_multiple_of(RECHERCHE_TOUS_LES);
            self.tours = self.tours.wrapping_add(1);
            let mut evenements = Vec::new();
            for (index, etat) in self.etats.iter_mut().enumerate() {
                if etat.is_none() && !chercher {
                    continue;
                }
                let mut brut: XINPUT_STATE = unsafe { std::mem::zeroed() };
                let resultat = unsafe { XInputGetState(index as u32, &mut brut) };
                if resultat != 0 {
                    if etat.take().is_some() {
                        evenements.push(Evenement::Deconnectee);
                    }
                    continue;
                }
                let g = brut.Gamepad;
                let nouveau = Etat {
                    boutons: g.wButtons,
                    gachette_gauche: g.bLeftTrigger,
                    gachette_droite: g.bRightTrigger,
                    stick_x: g.sThumbLX,
                    stick_y: g.sThumbLY,
                };
                match etat {
                    None => {
                        evenements.push(Evenement::Connectee(format!(
                            "manette XInput n°{}",
                            index + 1
                        )));
                        evenements.extend(differences(&Etat::default(), &nouveau));
                        *etat = Some((brut.dwPacketNumber, nouveau));
                    }
                    // Le numéro de paquet change seulement quand la manette a changé.
                    Some((paquet, ancien)) if *paquet != brut.dwPacketNumber => {
                        evenements.extend(differences(ancien, &nouveau));
                        *etat = Some((brut.dwPacketNumber, nouveau));
                    }
                    Some(_) => {}
                }
            }
            evenements
        }
    }
}

#[cfg(not(windows))]
pub use autres::Manette;

#[cfg(not(windows))]
mod autres {
    use super::Evenement;
    use gilrs::{Axis, EventType, Gilrs};

    pub struct Manette {
        gilrs: Gilrs,
    }

    impl Manette {
        pub fn new() -> Result<Manette, String> {
            Gilrs::new()
                .map(|gilrs| Manette { gilrs })
                .map_err(|e| format!("impossible d'initialiser la lecture des manettes : {e}"))
        }

        pub fn noms(&self) -> Vec<String> {
            self.gilrs
                .gamepads()
                .map(|(_, g)| g.name().to_string())
                .collect()
        }

        pub fn lire(&mut self) -> Vec<Evenement> {
            let mut evenements = Vec::new();
            while let Some(event) = self.gilrs.next_event() {
                match event.event {
                    EventType::ButtonPressed(bouton, _) => {
                        evenements.push(Evenement::Appui(nom(bouton)))
                    }
                    EventType::ButtonReleased(bouton, _) => {
                        evenements.push(Evenement::Relache(nom(bouton)))
                    }
                    EventType::AxisChanged(Axis::LeftStickX, v, _) => {
                        evenements.push(Evenement::StickX(v))
                    }
                    EventType::AxisChanged(Axis::LeftStickY, v, _) => {
                        evenements.push(Evenement::StickY(v))
                    }
                    EventType::Connected => evenements.push(Evenement::Connectee(
                        self.gilrs.gamepad(event.id).name().to_string(),
                    )),
                    EventType::Disconnected => evenements.push(Evenement::Deconnectee),
                    _ => {}
                }
            }
            evenements
        }
    }

    /// Le nom gilrs d'un bouton, en texte fixe.
    fn nom(bouton: gilrs::Button) -> &'static str {
        use gilrs::Button::*;
        match bouton {
            South => "South",
            East => "East",
            North => "North",
            West => "West",
            LeftTrigger => "LeftTrigger",
            LeftTrigger2 => "LeftTrigger2",
            RightTrigger => "RightTrigger",
            RightTrigger2 => "RightTrigger2",
            Select => "Select",
            Start => "Start",
            Mode => "Mode",
            LeftThumb => "LeftThumb",
            RightThumb => "RightThumb",
            DPadUp => "DPadUp",
            DPadDown => "DPadDown",
            DPadLeft => "DPadLeft",
            DPadRight => "DPadRight",
            _ => "Autre",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appui_et_relache_des_boutons() {
        let avant = Etat::default();
        let apres = Etat {
            boutons: 0x1000 | 0x0004, // A (South) et gauche
            ..Etat::default()
        };
        assert_eq!(
            differences(&avant, &apres),
            vec![Evenement::Appui("DPadLeft"), Evenement::Appui("South")]
        );
        assert_eq!(
            differences(&apres, &avant),
            vec![Evenement::Relache("DPadLeft"), Evenement::Relache("South")]
        );
    }

    #[test]
    fn gachette_avec_hysteresis() {
        let relachee = Etat::default();
        let mi_course = Etat {
            gachette_droite: 110,
            ..Etat::default()
        };
        let appuyee = Etat {
            gachette_droite: 255,
            ..Etat::default()
        };
        // 110 ne suffit pas pour appuyer...
        assert!(differences(&relachee, &mi_course).is_empty());
        assert_eq!(
            differences(&relachee, &appuyee),
            vec![Evenement::Appui("RightTrigger2")]
        );
        // ...mais ne relâche pas non plus une gâchette déjà appuyée.
        assert!(differences(&appuyee, &mi_course).is_empty());
        assert_eq!(
            differences(&appuyee, &relachee),
            vec![Evenement::Relache("RightTrigger2")]
        );
    }

    #[test]
    fn stick() {
        let apres = Etat {
            stick_x: -32767,
            ..Etat::default()
        };
        assert_eq!(
            differences(&Etat::default(), &apres),
            vec![Evenement::StickX(-1.0)]
        );
    }
}
