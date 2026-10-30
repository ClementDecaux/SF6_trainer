//! Les différentes façons d'écrire un coup : numpad (« 236HP »), anglais (« qcf+HP »),
//! flèches (« ↓↘→ HP ») ou icônes de SF6 (flèches et pastilles poing / pied colorées).
//!
//! En interne, l'outil garde toujours la forme numpad (`combo::normaliser`). La syntaxe
//! choisie (réglages, ou `syntaxe = "..."` dans `overlay.toml`) ne change que l'affichage.

use eframe::egui::{self, Color32, RichText};
use serde::Deserialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Syntaxe {
    /// 2MK, 236HP, 63214MP
    Numpad,
    /// cr.MK, qcf+HP, hcb+MP
    Anglais,
    /// ↓ MK, ↓↘→ HP
    Fleches,
    /// Flèches + pastilles des boutons, comme dans le jeu.
    #[default]
    Sf6,
}

impl Syntaxe {
    pub const TOUTES: [Syntaxe; 4] = [
        Syntaxe::Sf6,
        Syntaxe::Numpad,
        Syntaxe::Anglais,
        Syntaxe::Fleches,
    ];

    /// Le nom affiché dans les réglages.
    pub fn nom(self) -> &'static str {
        match self {
            Syntaxe::Numpad => "Numpad (2MK, 236HP)",
            Syntaxe::Anglais => "Anglais (cr.MK, qcf+HP)",
            Syntaxe::Fleches => "Flèches",
            Syntaxe::Sf6 => "SF6 (icônes du jeu)",
        }
    }

    /// Le mot écrit dans `overlay.toml`.
    pub fn cle(self) -> &'static str {
        match self {
            Syntaxe::Numpad => "numpad",
            Syntaxe::Anglais => "anglais",
            Syntaxe::Fleches => "fleches",
            Syntaxe::Sf6 => "sf6",
        }
    }
}

/// Un bouton de pastille : les six boutons, ou « un poing / un pied » (versions OD).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bouton {
    LP,
    MP,
    HP,
    LK,
    MK,
    HK,
    P,
    K,
}

impl Bouton {
    fn depuis(nom: &str) -> Option<Bouton> {
        Some(match nom {
            "LP" => Bouton::LP,
            "MP" => Bouton::MP,
            "HP" => Bouton::HP,
            "LK" => Bouton::LK,
            "MK" => Bouton::MK,
            "HK" => Bouton::HK,
            "P" => Bouton::P,
            "K" => Bouton::K,
            _ => return None,
        })
    }

    fn poing(self) -> bool {
        matches!(self, Bouton::LP | Bouton::MP | Bouton::HP | Bouton::P)
    }

    /// Couleurs du jeu : léger bleu, moyen jaune, fort rouge.
    fn couleur(self) -> Color32 {
        match self {
            Bouton::LP | Bouton::LK => Color32::from_rgb(40, 150, 235),
            Bouton::MP | Bouton::MK => Color32::from_rgb(240, 185, 20),
            Bouton::HP | Bouton::HK => Color32::from_rgb(225, 45, 50),
            Bouton::P | Bouton::K => Color32::from_rgb(150, 150, 165),
        }
    }
}

/// Un coup découpé en morceaux.
#[derive(Debug, PartialEq, Eq)]
pub enum Coup {
    /// Directions numpad (une seule pour un normal, plusieurs pour un mouvement) + boutons.
    Normal {
        directions: Vec<u8>,
        boutons: Vec<Bouton>,
    },
    DriveRush,
    /// Drive Impact (HP+HK).
    DriveImpact,
    /// Drive Parry (MP+MK).
    DriveParry,
    /// Choppe (LP+LK).
    Choppe,
    /// Pas reconnu : affiché tel quel.
    Autre(String),
}

/// Découpe une notation interne (« 236MK », « DR », « 5PP »...).
pub fn decouper(notation: &str) -> Coup {
    match notation.to_uppercase().as_str() {
        "DR" | "66" => return Coup::DriveRush,
        "DI" => return Coup::DriveImpact,
        "DP" => return Coup::DriveParry,
        "THROW" => return Coup::Choppe,
        _ => {}
    }
    let chiffres: String = notation
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let reste = &notation[chiffres.len()..];
    let boutons = match reste {
        "PP" => Some(vec![Bouton::P, Bouton::P]),
        "KK" => Some(vec![Bouton::K, Bouton::K]),
        _ => Bouton::depuis(reste).map(|b| vec![b]),
    };
    match boutons {
        Some(boutons) => Coup::Normal {
            directions: if chiffres.is_empty() {
                vec![5]
            } else {
                chiffres.bytes().map(|c| c - b'0').collect()
            },
            boutons,
        },
        None => Coup::Autre(notation.to_string()),
    }
}

fn nom_bouton(b: Bouton) -> &'static str {
    match b {
        Bouton::LP => "LP",
        Bouton::MP => "MP",
        Bouton::HP => "HP",
        Bouton::LK => "LK",
        Bouton::MK => "MK",
        Bouton::HK => "HK",
        Bouton::P => "P",
        Bouton::K => "K",
    }
}

fn boutons_texte(boutons: &[Bouton]) -> String {
    boutons.iter().map(|&b| nom_bouton(b)).collect()
}

pub fn fleche(direction: u8) -> &'static str {
    match direction {
        1 => "↙",
        2 => "↓",
        3 => "↘",
        4 => "←",
        6 => "→",
        7 => "↖",
        8 => "↑",
        9 => "↗",
        _ => "",
    }
}

/// Le coup en texte simple pour les endroits sans icônes : les polices de l'interface
/// n'ont pas les flèches, donc flèches et SF6 s'y écrivent en numpad.
pub fn texte_simple(notation: &str, syntaxe: Syntaxe) -> String {
    match syntaxe {
        Syntaxe::Fleches | Syntaxe::Sf6 => texte(notation, Syntaxe::Numpad),
        _ => texte(notation, syntaxe),
    }
}

/// Le coup écrit en texte dans la syntaxe choisie (SF6 : comme les flèches).
pub fn texte(notation: &str, syntaxe: Syntaxe) -> String {
    let coup = decouper(notation);
    match syntaxe {
        Syntaxe::Numpad => match coup {
            Coup::Choppe => "Throw".to_string(),
            _ => notation.to_string(),
        },
        Syntaxe::Anglais => match coup {
            Coup::Normal {
                directions,
                boutons,
            } => {
                let mouvement: String = directions.iter().map(|d| d.to_string()).collect();
                let prefixe = match mouvement.as_str() {
                    "5" => "st.",
                    "2" => "cr.",
                    "1" => "d/b+",
                    "3" => "d/f+",
                    "4" => "b+",
                    "6" => "f+",
                    "7" => "u/b+",
                    "8" => "u+",
                    "9" => "u/f+",
                    "236" => "qcf+",
                    "214" => "qcb+",
                    "41236" => "hcf+",
                    "63214" => "hcb+",
                    "623" => "dp+",
                    "421" => "rdp+",
                    _ => return notation.to_string(),
                };
                format!("{prefixe}{}", boutons_texte(&boutons))
            }
            Coup::DriveRush => "DR".to_string(),
            Coup::DriveImpact => "DI".to_string(),
            // « DP » veut dire Dragon Punch (623) en anglais.
            Coup::DriveParry => "Parry".to_string(),
            Coup::Choppe => "Throw".to_string(),
            Coup::Autre(t) => t,
        },
        Syntaxe::Fleches | Syntaxe::Sf6 => match coup {
            Coup::Normal {
                directions,
                boutons,
            } => {
                let fleches: String = directions.iter().map(|&d| fleche(d)).collect();
                if fleches.is_empty() {
                    boutons_texte(&boutons)
                } else {
                    format!("{fleches} {}", boutons_texte(&boutons))
                }
            }
            Coup::DriveRush => "DR".to_string(),
            Coup::DriveImpact => "DI".to_string(),
            Coup::DriveParry => "Parry".to_string(),
            Coup::Choppe => "Throw".to_string(),
            Coup::Autre(t) => t,
        },
    }
}

// ---------------------------------------------------------------- Liaisons

/// Comment afficher les liaisons entre deux coups (cancel / link).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StyleLiaison {
    /// Petites étiquettes colorées : CANCEL (orange), LINK (bleu).
    #[default]
    Pastilles,
    /// « cancel », « link » en toutes lettres.
    Mots,
    /// « xx » et « , » comme sur les forums.
    Symboles,
    /// « > » et « , ».
    Courts,
    /// Rien.
    Aucune,
}

impl StyleLiaison {
    pub const TOUS: [StyleLiaison; 5] = [
        StyleLiaison::Pastilles,
        StyleLiaison::Mots,
        StyleLiaison::Symboles,
        StyleLiaison::Courts,
        StyleLiaison::Aucune,
    ];

    pub fn nom(self) -> &'static str {
        match self {
            StyleLiaison::Pastilles => "Étiquettes (CANCEL, LINK)",
            StyleLiaison::Mots => "Mots (cancel, link)",
            StyleLiaison::Symboles => "Symboles (xx  ,)",
            StyleLiaison::Courts => "Courts (>  ,)",
            StyleLiaison::Aucune => "Aucune",
        }
    }

    pub fn cle(self) -> &'static str {
        match self {
            StyleLiaison::Pastilles => "pastilles",
            StyleLiaison::Mots => "mots",
            StyleLiaison::Symboles => "symboles",
            StyleLiaison::Courts => "courts",
            StyleLiaison::Aucune => "aucune",
        }
    }

    /// Largeur de la colonne des liaisons, pour que tous les coups soient alignés.
    fn largeur(self, taille: f32) -> f32 {
        let l = match self {
            StyleLiaison::Pastilles => 4.6,
            StyleLiaison::Mots => 3.6,
            StyleLiaison::Symboles => 1.7,
            StyleLiaison::Courts => 1.2,
            StyleLiaison::Aucune => 0.0,
        };
        (l * taille).round()
    }
}

pub const COULEUR_CANCEL: Color32 = Color32::from_rgb(255, 150, 40);
pub const COULEUR_LINK: Color32 = Color32::from_rgb(80, 185, 255);

/// Dessine la liaison avant un coup. `colonne` : largeur fixe (même pour le 1er coup, sans
/// liaison), pour aligner les coups les uns sous les autres.
pub fn dessiner_liaison(
    ui: &mut egui::Ui,
    liaison: crate::combo::Liaison,
    style: StyleLiaison,
    taille: f32,
    colonne: bool,
) {
    use crate::combo::Liaison;
    let cancel = match liaison {
        Liaison::Cancel => true,
        Liaison::Link => false,
        Liaison::Debut => {
            if colonne && style != StyleLiaison::Aucune {
                ui.allocate_exact_size(
                    egui::vec2(style.largeur(taille), taille),
                    egui::Sense::hover(),
                );
            }
            return;
        }
    };
    let (mot, couleur) = match (style, cancel) {
        (StyleLiaison::Aucune, _) => return,
        (StyleLiaison::Pastilles, true) => ("CANCEL", COULEUR_CANCEL),
        (StyleLiaison::Pastilles, false) => ("LINK", COULEUR_LINK),
        (StyleLiaison::Mots, true) => ("cancel", COULEUR_CANCEL),
        (StyleLiaison::Mots, false) => ("link", COULEUR_LINK),
        (StyleLiaison::Symboles, true) => ("xx", Color32::from_gray(160)),
        (StyleLiaison::Courts, true) => (">", Color32::from_gray(160)),
        (_, false) => (",", Color32::from_gray(160)),
    };
    let police = egui::FontId::proportional(if style == StyleLiaison::Pastilles {
        (taille * 0.8).round()
    } else {
        taille
    });
    let galee = ui
        .painter()
        .layout_no_wrap(mot.to_string(), police.clone(), couleur);
    let largeur = if colonne {
        style.largeur(taille).max(galee.size().x + 8.0)
    } else {
        galee.size().x
            + if style == StyleLiaison::Pastilles {
                10.0
            } else {
                2.0
            }
    };
    let (rect, _) = ui.allocate_exact_size(egui::vec2(largeur, taille * 1.2), egui::Sense::hover());
    let centre = rect.center();
    if style == StyleLiaison::Pastilles {
        let cadre = egui::Rect::from_center_size(
            centre,
            egui::vec2(galee.size().x + 8.0, galee.size().y + 2.0),
        );
        ui.painter().rect(
            cadre,
            4,
            couleur.gamma_multiply(0.18),
            egui::Stroke::new(1.0, couleur),
            egui::StrokeKind::Inside,
        );
    }
    ui.painter()
        .text(centre, egui::Align2::CENTER_CENTER, mot, police, couleur);
}

// ---------------------------------------------------------------- Dessin

/// Dessine un coup dans la syntaxe choisie, sur la ligne en cours (dans un `horizontal`).
/// `couleur` : couleur du texte et des flèches (les pastilles gardent leurs couleurs).
pub fn dessiner(
    ui: &mut egui::Ui,
    notation: &str,
    syntaxe: Syntaxe,
    taille: f32,
    couleur: Color32,
) {
    match syntaxe {
        Syntaxe::Numpad | Syntaxe::Anglais => {
            ui.label(
                RichText::new(texte(notation, syntaxe))
                    .size(taille)
                    .strong()
                    .color(couleur),
            );
        }
        Syntaxe::Fleches | Syntaxe::Sf6 => {
            let espace = ui.spacing().item_spacing.x;
            ui.spacing_mut().item_spacing.x = (taille * 0.08).round();
            dessiner_icones(ui, notation, syntaxe, taille, couleur);
            ui.spacing_mut().item_spacing.x = espace;
            ui.add_space((taille * 0.15).round());
        }
    }
}

fn dessiner_icones(
    ui: &mut egui::Ui,
    notation: &str,
    syntaxe: Syntaxe,
    taille: f32,
    couleur: Color32,
) {
    let sf6 = syntaxe == Syntaxe::Sf6;
    let boutons = |ui: &mut egui::Ui, liste: &[Bouton]| {
        for (i, &b) in liste.iter().enumerate() {
            if sf6 {
                if i > 0 {
                    plus(ui, taille, couleur);
                }
                pastille(ui, b, taille, true);
            } else {
                ui.label(
                    RichText::new(if i > 0 {
                        format!("+{}", nom_bouton(b))
                    } else {
                        nom_bouton(b).to_string()
                    })
                    .size(taille)
                    .strong()
                    .color(couleur),
                );
            }
        }
    };
    match decouper(notation) {
        Coup::Normal {
            directions,
            boutons: liste,
        } => {
            for &d in directions.iter().filter(|&&d| d != 5) {
                dessiner_fleche(ui, d, taille, couleur);
            }
            if directions.iter().any(|&d| d != 5) {
                ui.add_space((taille * 0.12).round());
            }
            boutons(ui, &liste);
        }
        Coup::DriveRush => badge_texte(ui, "DR", taille, Color32::from_rgb(70, 200, 90)),
        Coup::DriveImpact => {
            if sf6 {
                boutons(ui, &[Bouton::HP, Bouton::HK]);
            } else {
                badge_texte(ui, "DI", taille, Color32::from_rgb(240, 200, 60));
            }
        }
        Coup::DriveParry => {
            if sf6 {
                boutons(ui, &[Bouton::MP, Bouton::MK]);
            } else {
                badge_texte(ui, "Parry", taille, Color32::from_rgb(110, 230, 120));
            }
        }
        Coup::Choppe => {
            if sf6 {
                boutons(ui, &[Bouton::LP, Bouton::LK]);
            } else {
                badge_texte(ui, "Throw", taille, Color32::from_rgb(230, 120, 230));
            }
        }
        Coup::Autre(t) => {
            ui.label(RichText::new(t).size(taille).strong().color(couleur));
        }
    }
}

/// Un « + » entre deux pastilles.
fn plus(ui: &mut egui::Ui, taille: f32, couleur: Color32) {
    ui.label(RichText::new("+").size(taille * 0.6).color(couleur));
}

/// Un mot en couleur (DR, DI...).
fn badge_texte(ui: &mut egui::Ui, mot: &str, taille: f32, couleur: Color32) {
    ui.label(RichText::new(mot).size(taille).strong().color(couleur));
}

/// Une flèche dessinée (pas un caractère : toutes les polices n'ont pas les diagonales).
pub fn dessiner_fleche(ui: &mut egui::Ui, direction: u8, taille: f32, couleur: Color32) {
    let cote = (taille * 1.05).round();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(cote, cote), egui::Sense::hover());
    let (dx, dy): (f32, f32) = match direction {
        1 => (-1.0, 1.0),
        2 => (0.0, 1.0),
        3 => (1.0, 1.0),
        4 => (-1.0, 0.0),
        6 => (1.0, 0.0),
        7 => (-1.0, -1.0),
        8 => (0.0, -1.0),
        9 => (1.0, -1.0),
        _ => return,
    };
    let d = egui::vec2(dx, dy).normalized();
    let centre = rect.center();
    let longueur = cote * 0.78;
    let pointe = centre + d * longueur * 0.5;
    let queue = centre - d * longueur * 0.5;
    let epaisseur = (taille * 0.13).max(2.0);
    let n = egui::vec2(-d.y, d.x);
    let tete = longueur * 0.45;
    let base = pointe - d * tete;
    ui.painter()
        .line_segment([queue, base], egui::Stroke::new(epaisseur, couleur));
    ui.painter().add(egui::Shape::convex_polygon(
        vec![pointe, base + n * tete * 0.6, base - n * tete * 0.6],
        couleur,
        egui::Stroke::NONE,
    ));
}

/// Une pastille ronde de bouton, comme dans SF6 : couleur de la force, poing ou pied.
/// `appuye` = false : plus pâle (bouton simplement tenu).
pub fn pastille(ui: &mut egui::Ui, bouton: Bouton, taille: f32, appuye: bool) {
    let cote = (taille * 1.3).round();
    let (rect, reponse) = ui.allocate_exact_size(egui::vec2(cote, cote), egui::Sense::hover());
    let centre = rect.center();
    let rayon = cote * 0.5;
    let alpha = if appuye { 255 } else { 110 };
    let fond = bouton.couleur();
    let fond = Color32::from_rgba_unmultiplied(fond.r(), fond.g(), fond.b(), alpha);
    let peintre = ui.painter();
    peintre.circle_filled(centre, rayon, Color32::from_black_alpha(alpha));
    peintre.circle_filled(centre, rayon * 0.86, fond);
    // Reflet en haut, pour l'effet bouton.
    peintre.circle_filled(
        centre - egui::vec2(0.0, rayon * 0.3),
        rayon * 0.45,
        Color32::from_white_alpha(if appuye { 40 } else { 15 }),
    );
    let (octets, uri): (&'static [u8], &str) = if bouton.poing() {
        (
            include_bytes!("../assets/icones/poing.svg"),
            "bytes://poing.svg",
        )
    } else {
        (
            include_bytes!("../assets/icones/pied.svg"),
            "bytes://pied.svg",
        )
    };
    let glyphe = egui::Rect::from_center_size(centre, egui::vec2(rayon * 1.4, rayon * 1.4));
    egui::Image::new(egui::ImageSource::Bytes {
        uri: uri.into(),
        bytes: octets.into(),
    })
    .tint(Color32::from_white_alpha(alpha))
    .paint_at(ui, glyphe);
    reponse.on_hover_text(nom_bouton(bouton));
}

/// Pastille d'une action de la manette (historique) : LP..HK en pastille, le reste en texte.
pub fn pastille_action(
    ui: &mut egui::Ui,
    action: crate::controles::Action,
    taille: f32,
    appuye: bool,
) -> bool {
    match Bouton::depuis(&action.to_string()) {
        Some(b) => {
            pastille(ui, b, taille, appuye);
            true
        }
        None => false,
    }
}

/// Un coup cliquable (cadre autour), par exemple les étapes du lecteur vidéo.
pub fn bouton_coup(
    ui: &mut egui::Ui,
    notation: &str,
    syntaxe: Syntaxe,
    taille: f32,
    couleur: Color32,
) -> egui::Response {
    let cadre = egui::Frame::new()
        .fill(ui.visuals().widgets.inactive.bg_fill)
        .corner_radius(5)
        .inner_margin(egui::Margin::symmetric(6, 3))
        .show(ui, |ui| {
            ui.horizontal(|ui| dessiner(ui, notation, syntaxe, taille, couleur));
        });
    let reponse = cadre.response.interact(egui::Sense::click());
    if reponse.hovered() {
        ui.painter().rect_stroke(
            reponse.rect,
            5,
            egui::Stroke::new(1.0, Color32::from_white_alpha(120)),
            egui::StrokeKind::Inside,
        );
    }
    reponse
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_coups_s_ecrivent_dans_chaque_syntaxe() {
        assert_eq!(texte("236MK", Syntaxe::Numpad), "236MK");
        assert_eq!(texte("236MK", Syntaxe::Anglais), "qcf+MK");
        assert_eq!(texte("2MK", Syntaxe::Anglais), "cr.MK");
        assert_eq!(texte("5HP", Syntaxe::Anglais), "st.HP");
        assert_eq!(texte("1HP", Syntaxe::Anglais), "d/b+HP");
        assert_eq!(texte("63214MP", Syntaxe::Anglais), "hcb+MP");
        assert_eq!(texte("236KK", Syntaxe::Anglais), "qcf+KK");
        assert_eq!(texte("DP", Syntaxe::Anglais), "Parry");
        assert_eq!(texte("236MK", Syntaxe::Fleches), "↓↘→ MK");
        assert_eq!(texte("5LP", Syntaxe::Fleches), "LP");
        assert_eq!(texte("DR", Syntaxe::Fleches), "DR");
    }

    #[test]
    fn decouper_les_coups() {
        assert_eq!(
            decouper("214PP"),
            Coup::Normal {
                directions: vec![2, 1, 4],
                boutons: vec![Bouton::P, Bouton::P]
            }
        );
        assert_eq!(decouper("Throw"), Coup::Choppe);
        assert_eq!(decouper("bidule"), Coup::Autre("bidule".into()));
    }
}
