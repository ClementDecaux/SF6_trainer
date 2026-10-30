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
    /// La souris est au-dessus de l'historique : la fenêtre la garde (pour la molette).
    survol: bool,
    /// Où sont l'historique et les boutons à l'écran (en points) : quand la souris les
    /// survole, la fenêtre la garde (molette, clics).
    zones: Vec<egui::Rect>,
    /// Nombre de lignes déjà vues : une nouvelle ligne ramène l'historique en haut.
    lignes_vues: u64,
    /// Mode déplacement : la fenêtre en train d'être glissée ou redimensionnée.
    glisse: Option<Glisse>,
}

/// Un glisser en cours : d'où il est parti, et quels bords il tire (aucun = déplacer).
struct Glisse {
    curseur: egui::Pos2,
    fenetre: egui::Rect,
    bords: Bords,
}

/// Les bords de la fenêtre sous la souris (pour la redimensionner).
#[derive(Clone, Copy, Default, PartialEq)]
struct Bords {
    gauche: bool,
    droite: bool,
    haut: bool,
    bas: bool,
}

impl Bords {
    /// Largeur (en points) de la bande, le long des bords, qui redimensionne.
    const BANDE: f32 = 12.0;

    fn sous(souris: egui::Pos2, fenetre: egui::Rect) -> Bords {
        Bords {
            gauche: souris.x < fenetre.min.x + Self::BANDE,
            droite: souris.x > fenetre.max.x - Self::BANDE,
            haut: souris.y < fenetre.min.y + Self::BANDE,
            bas: souris.y > fenetre.max.y - Self::BANDE,
        }
    }

    fn curseur(self) -> egui::CursorIcon {
        use egui::CursorIcon as C;
        match (self.gauche, self.droite, self.haut, self.bas) {
            (true, _, true, _) | (_, true, _, true) => C::ResizeNwSe,
            (true, _, _, true) | (_, true, true, _) => C::ResizeNeSw,
            (true, _, _, _) | (_, true, _, _) => C::ResizeHorizontal,
            (_, _, true, _) | (_, _, _, true) => C::ResizeVertical,
            _ => C::Grab,
        }
    }

    /// La fenêtre de départ, avec les bords tirés de `delta` (ou déplacée si aucun bord).
    fn appliquer(self, depart: egui::Rect, delta: egui::Vec2) -> egui::Rect {
        const MIN: egui::Vec2 = egui::vec2(220.0, 200.0);
        if self == Bords::default() {
            return depart.translate(delta);
        }
        let mut r = depart;
        if self.gauche {
            r.min.x = (depart.min.x + delta.x).min(depart.max.x - MIN.x);
        }
        if self.droite {
            r.max.x = (depart.max.x + delta.x).max(depart.min.x + MIN.x);
        }
        if self.haut {
            r.min.y = (depart.min.y + delta.y).min(depart.max.y - MIN.y);
        }
        if self.bas {
            r.max.y = (depart.max.y + delta.y).max(depart.min.y + MIN.y);
        }
        r
    }
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
            survol: false,
            zones: Vec::new(),
            lignes_vues: 0,
            glisse: None,
        }
    }

    /// Les clics et la molette passent au jeu, sauf en mode déplacement (F8)
    /// et quand la souris est au-dessus de l'historique (pour le faire défiler).
    fn traversant(&self) -> bool {
        !self.deplacement && !self.survol
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
        let mut partage = match self.partage.lock() {
            Ok(p) => p,
            Err(_) => return,
        };

        // F8 : on rend la fenêtre cliquable (pour la glisser), ou de nouveau traversable.
        // Pareil quand la souris arrive au-dessus de l'historique (pour la molette).
        let traversant_avant = self.traversant();
        if partage.deplacement != self.deplacement {
            self.deplacement = partage.deplacement;
            if !self.deplacement {
                enregistrer_position(ui.ctx());
            }
        }
        self.survol = self.zones.iter().any(|&z| souris_sur(ui.ctx(), Some(z)));
        if self.traversant() != traversant_avant {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::MousePassthrough(self.traversant()));
            // Le changement est appliqué après cette image : on règle Windows deux images plus tard.
            self.regler_dans = 2;
        }
        // La souris ne nous est pas envoyée quand elle traverse : on regarde où elle est
        // régulièrement.
        ui.ctx().request_repaint_after(Duration::from_millis(100));
        if self.regler_dans > 0 {
            self.regler_dans -= 1;
            if self.regler_dans == 0 {
                #[cfg(windows)]
                regler_fenetre_windows(frame, self.opacite_fenetre, self.traversant());
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
                        RichText::new("Glisse-moi, tire un bord pour la taille, F8 pour finir")
                            .color(JAUNE)
                            .size(16.0)
                            .strong(),
                    );
                }
                let fenetre = ui.ctx().input(|i| i.viewport().inner_rect);
                let ecran = |r: egui::Rect| fenetre.map(|f| r.translate(f.min.to_vec2()));
                self.zones.clear();
                let barre = boutons(ui, &mut partage);
                self.zones.extend(ecran(barre));
                entete(ui, &partage);
                ui.add_space(8.0);
                etapes(ui, &partage);
                if let Some(raison) = &partage.raison_estimee {
                    ui.label(
                        RichText::new(format!("frames estimées : {raison}"))
                            .color(JAUNE)
                            .size(13.0),
                    );
                }
                ui.add_space(8.0);
                message(ui, &partage);
                ui.separator();
                // L'historique défile à la molette ; une nouvelle ligne le ramène en haut.
                let mut defilement = egui::ScrollArea::vertical();
                if partage.lignes_total != self.lignes_vues {
                    self.lignes_vues = partage.lignes_total;
                    defilement = defilement.vertical_scroll_offset(0.0);
                }
                let sortie = defilement
                    .scroll_source(
                        egui::scroll_area::ScrollSource::MOUSE_WHEEL
                            | egui::scroll_area::ScrollSource::SCROLL_BAR,
                    )
                    .auto_shrink([false, false])
                    .show(ui, |ui| historique(ui, &partage.lignes));
                self.zones.extend(ecran(sortie.inner_rect));
            });

        // Mode déplacement : cliquer-glisser au milieu déplace la fenêtre, sur un bord
        // ou un coin il la redimensionne. On bouge la fenêtre nous-mêmes en suivant la
        // souris à l'écran : le glisser de Windows ne marche pas sur une fenêtre qui ne
        // prend pas le focus (pour le laisser à SF6).
        if self.deplacement {
            glisser(ui, &mut self.glisse);
        } else {
            self.glisse = None;
        }

        // Redessiner un peu plus tard pour faire disparaître le message à temps.
        if partage.message.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
    }
}

/// Mode déplacement : suit la souris pour déplacer ou redimensionner la fenêtre.
fn glisser(ui: &mut egui::Ui, glisse: &mut Option<Glisse>) {
    let ctx = ui.ctx().clone();
    let zone = ui.interact(
        ui.max_rect(),
        egui::Id::unique("deplacer"),
        egui::Sense::click_and_drag(),
    );
    let Some(fenetre) = ctx.input(|i| i.viewport().outer_rect) else {
        return;
    };
    let bords = match glisse {
        Some(g) => g.bords,
        None => zone
            .hover_pos()
            .map(|p| Bords::sous(p, ui.max_rect()))
            .unwrap_or_default(),
    };
    if zone.hovered() || glisse.is_some() {
        ctx.set_cursor_icon(bords.curseur());
    }
    if zone.drag_started()
        && let Some(curseur) = curseur_ecran(&ctx)
    {
        *glisse = Some(Glisse {
            curseur,
            fenetre,
            bords,
        });
    }
    if let Some(g) = glisse {
        if zone.dragged()
            && let Some(curseur) = curseur_ecran(&ctx)
        {
            let r = g.bords.appliquer(g.fenetre, curseur - g.curseur);
            if r.min != fenetre.min {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(r.min));
            }
            if r.size() != fenetre.size() {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(r.size()));
            }
            ctx.request_repaint();
        } else {
            *glisse = None;
        }
    }
}

/// La position de la souris sur l'écran (en points), même hors de la fenêtre.
#[cfg(windows)]
fn curseur_ecran(ctx: &egui::Context) -> Option<egui::Pos2> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut p = POINT { x: 0, y: 0 };
    if unsafe { GetCursorPos(&mut p) } == 0 {
        return None;
    }
    let echelle = ctx.pixels_per_point();
    Some(egui::pos2(p.x as f32 / echelle, p.y as f32 / echelle))
}

/// Ailleurs : la souris dans la fenêtre, ramenée à l'écran.
#[cfg(not(windows))]
fn curseur_ecran(ctx: &egui::Context) -> Option<egui::Pos2> {
    ctx.input(|i| {
        let souris = i.pointer.latest_pos()?;
        Some(souris + i.viewport().outer_rect?.min.to_vec2())
    })
}

/// Les boutons cliquables : son, vidéos, déplacement, photo. Renvoie leur zone.
fn boutons(ui: &mut egui::Ui, p: &mut Partage) -> egui::Rect {
    let icone = |octets: &'static [u8], nom: &'static str| {
        egui::Image::from_bytes(nom, octets).fit_to_exact_size(egui::vec2(20.0, 20.0))
    };
    let couleur = |actif: bool| if actif { Color32::WHITE } else { GRIS };
    ui.horizontal(|ui| {
        let son = crate::son::actif();
        let image = if son {
            icone(
                include_bytes!("../assets/icones/volume-high.svg"),
                "bytes://son.svg",
            )
        } else {
            icone(
                include_bytes!("../assets/icones/volume-off.svg"),
                "bytes://muet.svg",
            )
        };
        if ui
            .add(egui::Button::image(image.tint(couleur(son))).frame(false))
            .on_hover_text("Son (étapes et combo réussi)")
            .clicked()
        {
            crate::son::activer(!son);
        }

        let image = if p.enregistrement {
            icone(
                include_bytes!("../assets/icones/video.svg"),
                "bytes://video.svg",
            )
        } else {
            icone(
                include_bytes!("../assets/icones/video-off.svg"),
                "bytes://sans-video.svg",
            )
        };
        if ui
            .add(egui::Button::image(image.tint(couleur(p.enregistrement))).frame(false))
            .on_hover_text("Garder les vidéos des combos réussis (F9)")
            .clicked()
        {
            p.enregistrement = !p.enregistrement;
        }

        let image = icone(
            include_bytes!("../assets/icones/cursor-move.svg"),
            "bytes://deplacer.svg",
        );
        if ui
            .add(egui::Button::image(image.tint(couleur(p.deplacement))).frame(false))
            .on_hover_text("Déplacer l'overlay (F8)")
            .clicked()
        {
            p.deplacement = !p.deplacement;
        }

        let image = icone(
            include_bytes!("../assets/icones/camera.svg"),
            "bytes://photo.svg",
        );
        if ui
            .add(egui::Button::image(image.tint(Color32::WHITE)).frame(false))
            .on_hover_text("Photo du jeu en PNG (F10)")
            .clicked()
        {
            p.photo_demandee = true;
        }
    })
    .response
    .rect
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
            // Compteur de hits en direct, lu sur le panneau des dégâts du jeu.
            if p.degats_visibles {
                ui.label(
                    RichText::new(format!("{} hits", p.hits_direct))
                        .color(ROUGE)
                        .size(15.0)
                        .strong(),
                );
            }
        });
    });
}

/// La liste des étapes, une par ligne, comme les trials du jeu.
fn etapes(ui: &mut egui::Ui, p: &Partage) {
    for (i, etape) in p.etapes.iter().enumerate() {
        let cassee = p.etape_cassee == Some(i) && i >= p.index;
        let (couleur, fond) = if i < p.index {
            (VERT, Color32::TRANSPARENT)
        } else if cassee {
            // Là où le combo a cassé.
            (ROUGE, Color32::from_rgba_unmultiplied(255, 60, 60, 70))
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

        let bord = if cassee {
            egui::Stroke::new(1.5, ROUGE)
        } else {
            egui::Stroke::NONE
        };
        egui::Frame::new()
            .fill(fond)
            .stroke(bord)
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
        // Un raté reste affiché jusqu'au prochain essai : on a le temps de lire pourquoi.
        Some(m) if !m.reussi || m.quand.elapsed() < DUREE_MESSAGE => {
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
        // Là où le combo a cassé : fond et contour rouges.
        let (fond, bord) = if ligne.cassee {
            (
                Color32::from_rgba_unmultiplied(255, 60, 60, 70),
                egui::Stroke::new(1.5, ROUGE),
            )
        } else {
            (Color32::TRANSPARENT, egui::Stroke::NONE)
        };
        egui::Frame::new()
            .fill(fond)
            .stroke(bord)
            .corner_radius(4)
            .show(ui, |ui| ligne_historique(ui, ligne));
    }
}

/// Une ligne de l'historique : durée, direction, boutons et coups reconnus.
fn ligne_historique(ui: &mut egui::Ui, ligne: &Ligne) {
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
        if ligne.cassee {
            ui.label(RichText::new("× break").color(ROUGE).size(14.0).strong());
        }
    });
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
    let nouveau = remplacer_position(&texte, rect);
    if let Err(e) = std::fs::write(FICHIER, nouveau) {
        eprintln!("impossible d'écrire {FICHIER} : {e}");
    }
}

/// Remplace la position et la taille (`x`, `y`, `largeur`, `hauteur`) dans `overlay.toml`.
fn remplacer_position(texte: &str, rect: egui::Rect) -> String {
    let valeurs = [
        ("x", rect.min.x),
        ("y", rect.min.y),
        ("largeur", rect.width()),
        ("hauteur", rect.height()),
    ];
    let mut sortie = String::new();
    for ligne in texte.lines() {
        let cle = ligne.split('=').next().unwrap_or("").trim();
        match valeurs
            .iter()
            .find(|(nom, _)| *nom == cle && ligne.contains('='))
        {
            Some((nom, valeur)) => sortie.push_str(&format!("{nom} = {}", valeur.round())),
            None => sortie.push_str(ligne),
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
        WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TRANSPARENT,
    };

    let Ok(handle) = frame.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return;
    };
    let hwnd = win32.hwnd.get() as HWND;
    unsafe {
        // NOACTIVATE : cliquer sur l'overlay ne prend pas le focus à SF6.
        let mut style =
            GetWindowLongPtrW(hwnd, GWL_EXSTYLE) | (WS_EX_LAYERED | WS_EX_NOACTIVATE) as isize;
        if traversant {
            style |= WS_EX_TRANSPARENT as isize;
        } else {
            style &= !(WS_EX_TRANSPARENT as isize);
        }
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style);
        SetLayeredWindowAttributes(hwnd, 0, opacite, LWA_ALPHA);
    }
}

/// Vrai si la souris est dans `zone` (coordonnées de l'écran, en points).
#[cfg(windows)]
fn souris_sur(ctx: &egui::Context, zone: Option<egui::Rect>) -> bool {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let Some(zone) = zone else {
        return false;
    };
    let mut p = POINT { x: 0, y: 0 };
    if unsafe { GetCursorPos(&mut p) } == 0 {
        return false;
    }
    let echelle = ctx.pixels_per_point();
    zone.contains(egui::pos2(p.x as f32 / echelle, p.y as f32 / echelle))
}

#[cfg(not(windows))]
fn souris_sur(_ctx: &egui::Context, _zone: Option<egui::Rect>) -> bool {
    false
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
        let rect = egui::Rect::from_min_size(egui::pos2(640.0, 75.0), egui::vec2(420.0, 650.0));
        let nouveau = remplacer_position(CONFIG_PAR_DEFAUT, rect);
        let c: Config = toml::from_str(&nouveau).unwrap();
        assert_eq!((c.x, c.y), (640.0, 75.0));
        assert_eq!((c.largeur, c.hauteur), (420.0, 650.0));
        assert!(nouveau.contains("# Position et taille"));
    }

    #[test]
    fn tirer_un_bord_redimensionne() {
        let depart = egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(380.0, 720.0));
        let gauche = Bords {
            gauche: true,
            ..Bords::default()
        };
        let r = gauche.appliquer(depart, egui::vec2(-50.0, 30.0));
        assert_eq!((r.min.x, r.max.x, r.min.y), (50.0, 480.0, 100.0));
        // Taille minimale respectée.
        let r = gauche.appliquer(depart, egui::vec2(500.0, 0.0));
        assert_eq!(r.width(), 220.0);
        // Aucun bord : la fenêtre se déplace.
        let r = Bords::default().appliquer(depart, egui::vec2(10.0, 20.0));
        assert_eq!(r.size(), depart.size());
        assert_eq!(r.min, egui::pos2(110.0, 120.0));
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
