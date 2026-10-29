//! La fenêtre de l'overlay : transparente, toujours au premier plan, et les clics
//! passent au travers (on peut continuer à jouer).
//!
//! Elle affiche le trial (étapes faites en vert, étape attendue en jaune),
//! le dernier résultat, et l'historique d'inputs façon SF6.
//!
//! Les polices d'egui n'ont pas les symboles ✓ ou ↙ : on dessine les coches et les
//! flèches nous-mêmes avec le `Painter` (des traits, comme sur une feuille).

use crate::combo::Liaison;
use crate::controles::Action;
use crate::partage::{Ligne, Partage};
use crate::timings::verifier;
use eframe::egui::{self, Color32, RichText};
use serde::Deserialize;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const FICHIER: &str = "overlay.toml";

const CONFIG_PAR_DEFAUT: &str = r#"# Position et taille de l'overlay, en pixels depuis le coin haut gauche de l'écran.
# SF6 doit être en « plein écran fenêtré » (borderless) pour que l'overlay passe par-dessus.
x = 20
y = 120
largeur = 380
hauteur = 720
# Opacité du fond, de 0 (invisible) à 255 (noir opaque).
opacite_fond = 170
# Opacité de toute la fenêtre (texte compris), de 0 à 255. Sous Windows seulement.
opacite_fenetre = 230
# true = fenêtre transparente (on voit le jeu derrière le fond).
# false = fond sombre opaque : moins joli, mais marche partout.
transparence = false
# Petit son quand un combo est réussi.
son = true
"#;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub x: f32,
    pub y: f32,
    pub largeur: f32,
    pub hauteur: f32,
    pub opacite_fond: u8,
    /// Absent des anciens fichiers : `false` par défaut.
    #[serde(default)]
    pub transparence: bool,
    /// Absent des anciens fichiers : 230 par défaut.
    #[serde(default = "opacite_fenetre_par_defaut")]
    pub opacite_fenetre: u8,
    /// Absent des anciens fichiers : `true` par défaut.
    #[serde(default = "vrai")]
    pub son: bool,
}

fn vrai() -> bool {
    true
}

fn opacite_fenetre_par_defaut() -> u8 {
    230
}

/// Charge `overlay.toml`, ou le crée avec les valeurs par défaut.
pub fn charger() -> Result<Config, String> {
    let texte = match std::fs::read_to_string(FICHIER) {
        Ok(texte) => texte,
        Err(_) => {
            std::fs::write(FICHIER, CONFIG_PAR_DEFAUT)
                .map_err(|e| format!("impossible de créer {FICHIER} : {e}"))?;
            println!("{FICHIER} créé : modifie x et y pour déplacer l'overlay.");
            CONFIG_PAR_DEFAUT.to_string()
        }
    };
    toml::from_str(&texte).map_err(|e| format!("erreur dans {FICHIER} : {e}"))
}

/// Combien de temps le message « réussi » ou « raté » reste affiché.
const DUREE_MESSAGE: Duration = Duration::from_millis(2000);

const VERT: Color32 = Color32::from_rgb(110, 230, 120);
const JAUNE: Color32 = Color32::from_rgb(255, 210, 60);
const ROUGE: Color32 = Color32::from_rgb(255, 100, 100);
const CYAN: Color32 = Color32::from_rgb(100, 220, 230);
const GRIS: Color32 = Color32::from_rgb(150, 150, 150);

pub struct Overlay {
    partage: Arc<Mutex<Partage>>,
    opacite_fond: u8,
    transparence: bool,
    opacite_fenetre: u8,
    /// Dans combien d'images régler la fenêtre côté Windows (0 = rien à faire).
    /// Au démarrage, et juste après chaque changement du mode déplacement.
    regler_dans: u8,
    /// Mode déplacement actuellement appliqué à la fenêtre.
    deplacement: bool,
}

impl Overlay {
    pub fn new(partage: Arc<Mutex<Partage>>, config: &Config) -> Overlay {
        Overlay {
            partage,
            opacite_fond: config.opacite_fond,
            transparence: config.transparence,
            opacite_fenetre: config.opacite_fenetre,
            regler_dans: 1,
            deplacement: false,
        }
    }
}

impl eframe::App for Overlay {
    /// Fond de la fenêtre : entièrement transparent (seul notre cadre se voit),
    /// ou gris très sombre si la transparence est désactivée.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        if self.transparence {
            [0.0, 0.0, 0.0, 0.0]
        } else {
            [0.06, 0.06, 0.06, 1.0]
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        // On verrouille le partage le temps de dessiner (quelques microsecondes).
        let partage = match self.partage.lock() {
            Ok(p) => p,
            Err(_) => return,
        };

        // F8 : on rend la fenêtre cliquable (pour la glisser), ou de nouveau traversable.
        if partage.deplacement != self.deplacement {
            self.deplacement = partage.deplacement;
            let ctx = ui.ctx().clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(!self.deplacement));
            if !self.deplacement {
                enregistrer_position(&ctx);
            }
            // Le changement est appliqué après cette image : on règle Windows deux images plus tard.
            self.regler_dans = 2;
        }
        if self.regler_dans > 0 {
            self.regler_dans -= 1;
            if self.regler_dans == 0 {
                #[cfg(windows)]
                regler_fenetre_windows(frame, self.opacite_fenetre, !self.deplacement);
                #[cfg(not(windows))]
                let _ = (frame, self.opacite_fenetre);
            } else {
                ui.ctx().request_repaint();
            }
        }

        let bordure = if self.deplacement {
            egui::Stroke::new(3.0, JAUNE)
        } else {
            egui::Stroke::NONE
        };
        egui::Frame::new()
            .fill(Color32::from_rgba_unmultiplied(0, 0, 0, self.opacite_fond))
            .stroke(bordure)
            .corner_radius(10)
            .inner_margin(14)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if self.deplacement {
                    ui.label(
                        RichText::new("Glisse-moi à la souris, F8 pour finir")
                            .color(JAUNE)
                            .size(16.0)
                            .strong(),
                    );
                }
                entete(ui, &partage);
                ui.add_space(8.0);
                etapes(ui, &partage);
                ui.add_space(8.0);
                message(ui, &partage);
                ui.separator();
                historique(ui, &partage.lignes);
            });

        // Mode déplacement : cliquer-glisser n'importe où déplace la fenêtre.
        if self.deplacement {
            let zone = ui.interact(
                ui.max_rect(),
                egui::Id::unique("deplacer"),
                egui::Sense::click_and_drag(),
            );
            if zone.drag_started() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
        }

        // Redessiner un peu plus tard pour faire disparaître le message à temps.
        if partage.message.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
    }
}

fn entete(ui: &mut egui::Ui, p: &Partage) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(&p.nom).color(GRIS).size(15.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                RichText::new(format!("{} réussi(s)", p.reussites))
                    .color(VERT)
                    .size(15.0),
            );
            if p.enregistrement {
                ui.label(RichText::new("● REC").color(ROUGE).size(15.0).strong());
            }
            // D'où viennent les frames : l'historique du jeu (exact) ou notre estimation.
            let (texte, couleur) = if p.historique_jeu {
                ("frames du jeu", VERT)
            } else {
                ("frames estimées", JAUNE)
            };
            ui.label(RichText::new(texte).color(couleur).size(13.0));
        });
    });
}

/// La liste des étapes, une par ligne, comme les trials du jeu.
fn etapes(ui: &mut egui::Ui, p: &Partage) {
    for (i, etape) in p.etapes.iter().enumerate() {
        let (couleur, fond) = if i < p.index {
            (VERT, Color32::TRANSPARENT)
        } else if i == p.index {
            (JAUNE, Color32::from_rgba_unmultiplied(255, 210, 60, 40))
        } else {
            (Color32::from_white_alpha(170), Color32::TRANSPARENT)
        };
        let liaison = match etape.liaison {
            Liaison::Debut => "",
            Liaison::Cancel => "xx",
            Liaison::Link => ",",
        };

        egui::Frame::new()
            .fill(fond)
            .corner_radius(6)
            .inner_margin(egui::Margin::symmetric(8, 3))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    dessiner_coche(ui, i < p.index);
                    ui.label(RichText::new(liaison).color(GRIS).size(14.0).monospace());
                    ui.label(
                        RichText::new(&etape.notation)
                            .color(couleur)
                            .size(24.0)
                            .strong(),
                    );
                    timing(ui, p, i);
                });
            });
    }
}

/// À droite d'une étape : son écart avec la précédente (« +14 f »), en rouge s'il sort
/// de la fenêtre apprise, et la fenêtre elle-même en petit.
fn timing(ui: &mut egui::Ui, p: &Partage, i: usize) {
    if i == 0 {
        return;
    }
    let fenetre = p.fenetres.as_ref().and_then(|f| f.get(i - 1).copied());
    let ecart = p.ecarts.get(i - 1).copied();
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if let Some((min, max)) = fenetre {
            let texte = if min == max {
                format!("{min}")
            } else {
                format!("{min}-{max}")
            };
            ui.label(RichText::new(texte).color(GRIS).size(13.0).monospace());
        }
        if let Some(ecart) = ecart {
            let hors = fenetre.is_some_and(|f| verifier(ecart, f).is_some());
            let couleur = if hors { ROUGE } else { CYAN };
            // « ~ » : écart estimé, pas encore vérifié sur l'historique du jeu.
            let approche = if p.ecarts_exacts { "" } else { "~" };
            ui.label(
                RichText::new(format!("{approche}+{ecart} f"))
                    .color(couleur)
                    .size(15.0)
                    .monospace(),
            );
        }
    });
}

/// « COMBO RÉUSSI » ou « attendu X, reçu Y », pendant 2 secondes.
fn message(ui: &mut egui::Ui, p: &Partage) {
    match &p.message {
        Some(m) if m.quand.elapsed() < DUREE_MESSAGE => {
            let couleur = if m.reussi { VERT } else { ROUGE };
            ui.label(RichText::new(&m.texte).color(couleur).size(22.0).strong());
        }
        // Ligne vide de la même hauteur, pour que rien ne saute quand le message apparaît.
        _ => {
            ui.label(RichText::new(" ").size(22.0));
        }
    }
}

/// L'historique d'inputs, le plus récent en haut comme dans SF6.
fn historique(ui: &mut egui::Ui, lignes: &std::collections::VecDeque<Ligne>) {
    for ligne in lignes {
        ui.horizontal(|ui| {
            let duree = if ligne.frames > 99 {
                "99+".to_string()
            } else {
                format!("{:>3}", ligne.frames)
            };
            // En cyan : durée du jeu. En gris : notre estimation (pas retrouvée dans le jeu).
            let couleur = if ligne.exact { CYAN } else { GRIS };
            ui.label(RichText::new(duree).color(couleur).size(16.0).monospace());

            let couleur_dir = if ligne.direction == 5 {
                GRIS
            } else {
                Color32::WHITE
            };
            dessiner_direction(ui, ligne.direction, couleur_dir);

            for &(action, nouveau) in &ligne.boutons {
                let mut texte = RichText::new(action.to_string())
                    .color(couleur_bouton(action))
                    .size(16.0);
                // Appuyé à ce moment : en gras. Simplement tenu : plus pâle.
                texte = if nouveau {
                    texte.strong()
                } else {
                    texte.weak()
                };
                ui.label(texte);
            }

            for coup in &ligne.coups {
                ui.label(RichText::new(format!("» {coup}")).color(JAUNE).size(16.0));
            }
        });
    }
}

/// Une flèche dans la direction numpad (un point pour le neutre).
fn dessiner_direction(ui: &mut egui::Ui, direction: u8, couleur: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
    let centre = rect.center();
    // Sur l'écran, y augmente vers le bas : « haut » = y négatif.
    let (dx, dy): (f32, f32) = match direction {
        1 => (-1.0, 1.0),
        2 => (0.0, 1.0),
        3 => (1.0, 1.0),
        4 => (-1.0, 0.0),
        6 => (1.0, 0.0),
        7 => (-1.0, -1.0),
        8 => (0.0, -1.0),
        9 => (1.0, -1.0),
        _ => {
            ui.painter().circle_filled(centre, 3.0, couleur);
            return;
        }
    };
    let vecteur = egui::vec2(dx, dy).normalized() * 15.0;
    ui.painter().arrow(
        centre - vecteur * 0.5,
        vecteur,
        egui::Stroke::new(2.5, couleur),
    );
}

/// Une coche verte pour une étape réussie (ou un espace vide de la même taille).
fn dessiner_coche(ui: &mut egui::Ui, visible: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
    if visible {
        let p = |x: f32, y: f32| rect.min + egui::vec2(x, y);
        ui.painter().line(
            vec![p(3.0, 10.0), p(8.0, 15.0), p(17.0, 4.0)],
            egui::Stroke::new(3.0, VERT),
        );
    }
}

/// Windows : rend le contenu visible malgré « les clics passent au travers ».
///
/// Pour laisser passer les clics, la fenêtre reçoit le style « layered » de Windows.
/// Une fenêtre layered n'affiche rien tant qu'on ne lui donne pas d'opacité :
/// c'est ce que fait `SetLayeredWindowAttributes`. Bonus : on règle ainsi la
/// transparence de toute la fenêtre.
///
/// `unsafe` : on appelle directement des fonctions C de Windows, Rust ne peut pas
/// vérifier lui-même qu'on les utilise correctement.
/// Écrit la position actuelle de la fenêtre dans `overlay.toml` (lignes `x =` et `y =`),
/// sans toucher au reste du fichier.
fn enregistrer_position(ctx: &egui::Context) {
    let Some(rect) = ctx.input(|i| i.viewport().outer_rect) else {
        return;
    };
    let Ok(texte) = std::fs::read_to_string(FICHIER) else {
        return;
    };
    let nouveau = remplacer_position(&texte, rect.min.x.round(), rect.min.y.round());
    if let Err(e) = std::fs::write(FICHIER, nouveau) {
        eprintln!("impossible d'écrire {FICHIER} : {e}");
    }
}

fn remplacer_position(texte: &str, x: f32, y: f32) -> String {
    let mut sortie = String::new();
    for ligne in texte.lines() {
        let debut = ligne.trim_start();
        if debut.starts_with("x =") || debut.starts_with("x=") {
            sortie.push_str(&format!("x = {x}"));
        } else if debut.starts_with("y =") || debut.starts_with("y=") {
            sortie.push_str(&format!("y = {y}"));
        } else {
            sortie.push_str(ligne);
        }
        sortie.push('\n');
    }
    sortie
}

/// `traversant` : les clics passent au travers de la fenêtre (sauf en mode déplacement).
#[cfg(windows)]
fn regler_fenetre_windows(frame: &eframe::Frame, opacite: u8, traversant: bool) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowLongPtrW, LWA_ALPHA, SetLayeredWindowAttributes, SetWindowLongPtrW,
        WS_EX_LAYERED, WS_EX_TRANSPARENT,
    };

    let Ok(handle) = frame.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return;
    };
    let hwnd = win32.hwnd.get() as HWND;
    unsafe {
        let mut style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) | WS_EX_LAYERED as isize;
        if traversant {
            style |= WS_EX_TRANSPARENT as isize;
        } else {
            style &= !(WS_EX_TRANSPARENT as isize);
        }
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style);
        SetLayeredWindowAttributes(hwnd, 0, opacite, LWA_ALPHA);
    }
}

/// Mêmes couleurs que dans le terminal : poings en bleu, pieds en rouge...
fn couleur_bouton(action: Action) -> Color32 {
    match action {
        Action::LP | Action::MP | Action::HP => Color32::from_rgb(100, 160, 255),
        Action::LK | Action::MK | Action::HK => Color32::from_rgb(255, 100, 100),
        Action::DP => Color32::from_rgb(110, 230, 120),
        Action::DI => Color32::from_rgb(240, 200, 60),
        Action::Throw => Color32::from_rgb(230, 120, 230),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_position_est_remplacee_sans_toucher_au_reste() {
        let nouveau = remplacer_position(CONFIG_PAR_DEFAUT, 640.0, 75.0);
        let c: Config = toml::from_str(&nouveau).unwrap();
        assert_eq!((c.x, c.y), (640.0, 75.0));
        assert_eq!(c.largeur, 380.0);
        assert!(nouveau.contains("# Position et taille"));
    }

    #[test]
    fn la_config_par_defaut_se_lit() {
        let c: Config = toml::from_str(CONFIG_PAR_DEFAUT).unwrap();
        assert_eq!(c.largeur, 380.0);
        assert_eq!(c.opacite_fond, 170);
        assert!(!c.transparence);
    }

    #[test]
    fn un_ancien_fichier_sans_transparence_se_lit() {
        let c: Config =
            toml::from_str("x = 20\ny = 120\nlargeur = 380\nhauteur = 720\nopacite_fond = 170\n")
                .unwrap();
        assert!(!c.transparence);
        assert!(c.son);
    }
}
