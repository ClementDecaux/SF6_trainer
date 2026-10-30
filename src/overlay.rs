//! La fenêtre de l'overlay : transparente, toujours au premier plan, et les clics
//! passent au travers (on peut continuer à jouer).
//!
//! Elle affiche le trial (étapes faites en vert, étape attendue en jaune),
//! le dernier résultat, et l'historique d'inputs façon SF6.
//!
//! Les polices d'egui n'ont pas les symboles ✓ ou ↙ : on dessine les coches et les
//! flèches nous-mêmes avec le `Painter` (des traits, comme sur une feuille).

use crate::controles::Action;
use crate::partage::{Ligne, Partage};
use crate::syntaxe::Syntaxe;
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
# Façon d'écrire les coups : "sf6" (icônes du jeu), "numpad" (236HP),
# "anglais" (qcf+HP) ou "fleches" (↓↘→ HP). Se change aussi dans les réglages.
syntaxe = "sf6"
# true = l'overlay n'apparaît que quand SF6 est la fenêtre active.
seulement_jeu = true
# true = l'overlay n'apparaît qu'en mode entraînement (ou dans un replay) : quand
# l'historique d'inputs ou les infos de dégâts de SF6 sont à l'écran. Pas en ligne, pas
# dans les menus. Les coups ne sont pas non plus suivis ailleurs.
seulement_entrainement = true
# Liaisons entre les coups : "pastilles" (CANCEL / LINK), "mots", "symboles" (xx ,),
# "courts" (> ,) ou "aucune".
liaisons = "pastilles"
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
    /// Façon d'écrire les coups. Absent des anciens fichiers : numpad.
    #[serde(default)]
    pub syntaxe: Syntaxe,
    /// L'overlay n'apparaît que quand SF6 est la fenêtre active. Par défaut : oui.
    #[serde(default = "vrai")]
    pub seulement_jeu: bool,
    /// L'overlay (et le suivi des coups) seulement en mode entraînement. Par défaut : oui.
    #[serde(default = "vrai")]
    pub seulement_entrainement: bool,
    /// Affichage des liaisons (cancel / link).
    #[serde(default)]
    pub liaisons: crate::syntaxe::StyleLiaison,
    /// L'historique est déplié (languette en bas de l'overlay).
    #[serde(default = "vrai")]
    pub historique_ouvert: bool,
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

pub(crate) const VERT: Color32 = Color32::from_rgb(110, 230, 120);
pub(crate) const JAUNE: Color32 = Color32::from_rgb(255, 210, 60);
pub(crate) const ROUGE: Color32 = Color32::from_rgb(255, 100, 100);
pub(crate) const CYAN: Color32 = Color32::from_rgb(100, 220, 230);
pub(crate) const GRIS: Color32 = Color32::from_rgb(150, 150, 150);

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
    /// La souris est au-dessus de l'overlay : la fenêtre la garde (clics, molette).
    survol: bool,
    /// Pas dans SF6 ou pas en mode entraînement : la fenêtre est invisible et traversée.
    cache: bool,
    /// L'historique est déplié (languette en bas).
    historique_ouvert: bool,
    /// Hauteur de la fenêtre historique déplié (repliée, la fenêtre se réduit au contenu).
    hauteur_ouverte: f32,
    /// Juste après avoir déplié : la hauteur qu'on attend que la fenêtre reprenne.
    attendre_hauteur: Option<f32>,
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
    /// La dernière place demandée pour la fenêtre.
    place: egui::Rect,
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
    const BANDE: f32 = 8.0;
    /// Taille des coins.
    const COIN: f32 = 16.0;

    /// Les zones qui redimensionnent : les 4 côtés et les 4 coins, chacune avec ses bords.
    fn zones(f: egui::Rect) -> Vec<(egui::Rect, Bords)> {
        let (b, c) = (Self::BANDE, Self::COIN);
        let bords = |gauche, droite, haut, bas| Bords {
            gauche,
            droite,
            haut,
            bas,
        };
        let r = |x0: f32, y0: f32, x1: f32, y1: f32| {
            egui::Rect::from_min_max(egui::pos2(x0, y0), egui::pos2(x1, y1))
        };
        vec![
            (
                r(f.min.x, f.min.y + c, f.min.x + b, f.max.y - c),
                bords(true, false, false, false),
            ),
            (
                r(f.max.x - b, f.min.y + c, f.max.x, f.max.y - c),
                bords(false, true, false, false),
            ),
            (
                r(f.min.x + c, f.min.y, f.max.x - c, f.min.y + b),
                bords(false, false, true, false),
            ),
            (
                r(f.min.x + c, f.max.y - b, f.max.x - c, f.max.y),
                bords(false, false, false, true),
            ),
            (
                r(f.min.x, f.min.y, f.min.x + c, f.min.y + c),
                bords(true, false, true, false),
            ),
            (
                r(f.max.x - c, f.min.y, f.max.x, f.min.y + c),
                bords(false, true, true, false),
            ),
            (
                r(f.min.x, f.max.y - c, f.min.x + c, f.max.y),
                bords(true, false, false, true),
            ),
            (
                r(f.max.x - c, f.max.y - c, f.max.x, f.max.y),
                bords(false, true, false, true),
            ),
        ]
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
            cache: false,
            historique_ouvert: config.historique_ouvert,
            hauteur_ouverte: config.hauteur,
            attendre_hauteur: None,
            lignes_vues: 0,
            glisse: None,
        }
    }

    /// Les clics et la molette passent au jeu, sauf en mode déplacement (F8)
    /// et quand la souris est au-dessus de l'historique (pour le faire défiler).
    fn traversant(&self) -> bool {
        self.cache || (!self.deplacement && !self.survol && self.glisse.is_none())
    }

    /// Montre ou rend invisible l'overlay. Vrai si ça change (il faut alors le redessiner).
    pub fn montrer(&mut self, oui: bool) -> bool {
        if self.cache != oui {
            return false;
        }
        self.cache = !oui;
        self.regler_dans = 1;
        true
    }
}

/// Le titre de la fenêtre de l'overlay (sert aussi à la retrouver côté Windows).
pub const TITRE: &str = "SF6 Trainer - overlay";

/// L'identifiant egui de la fenêtre de l'overlay.
pub fn id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("overlay")
}

/// La fenêtre de l'overlay, telle qu'elle est créée.
pub fn fenetre(config: &Config) -> egui::ViewportBuilder {
    egui::ViewportBuilder::default()
        .with_title(TITRE)
        .with_transparent(config.transparence)
        .with_decorations(false)
        .with_always_on_top()
        // Créée sans prendre le focus (SF6 le garde).
        .with_active(false)
        .with_mouse_passthrough(true)
        .with_taskbar(false)
        .with_position([config.x, config.y])
        .with_inner_size([config.largeur, config.hauteur])
}

impl Overlay {
    /// Fond de la fenêtre : entièrement transparent (seul notre cadre se voit),
    /// ou gris très sombre si la transparence est désactivée.
    pub fn fond(&self) -> [f32; 4] {
        if self.transparence {
            [0.0, 0.0, 0.0, 0.0]
        } else {
            [0.06, 0.06, 0.06, 1.0]
        }
    }

    /// Opacité du fond du cadre (0 à 255), réglable dans l'interface.
    pub fn opacite_fond(&self) -> u8 {
        self.opacite_fond
    }

    pub fn regler_opacite_fond(&mut self, opacite: u8) {
        self.opacite_fond = opacite;
    }

    /// Opacité de toute la fenêtre (0 à 255), réglable dans l'interface.
    pub fn opacite_fenetre(&self) -> u8 {
        self.opacite_fenetre
    }

    pub fn regler_opacite_fenetre(&mut self, opacite: u8) {
        if opacite != self.opacite_fenetre {
            self.opacite_fenetre = opacite;
            self.regler_dans = 1;
        }
    }

    /// La place à écrire dans overlay.toml : avec la hauteur dépliée, même si repliée.
    fn place_a_enregistrer(&self, mut rect: egui::Rect) -> egui::Rect {
        if !self.historique_ouvert {
            rect.set_height(self.hauteur_ouverte);
        }
        rect
    }

    /// La fenêtre vient d'être (re)créée : il faut la régler de nouveau côté Windows.
    pub fn fenetre_recreee(&mut self) {
        self.regler_dans = 2;
    }

    /// Dessine l'overlay dans sa fenêtre.
    pub fn dessiner(&mut self, ui: &mut egui::Ui) {
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
            if !self.deplacement
                && let Some(rect) = ui.ctx().input(|i| i.viewport().outer_rect)
            {
                enregistrer_rect(self.place_a_enregistrer(rect));
            }
        }
        // La souris au-dessus de l'overlay : la fenêtre la garde (boutons, molette, bande
        // du haut). Sinon elle irait au jeu, qui cache le curseur.
        let rect_fenetre = ui.ctx().input(|i| i.viewport().inner_rect);
        self.survol = souris_sur(ui.ctx(), rect_fenetre);
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
                let opacite = if self.cache { 0 } else { self.opacite_fenetre };
                #[cfg(windows)]
                regler_fenetre_windows(opacite, self.traversant());
                #[cfg(not(windows))]
                let _ = opacite;
            } else {
                ui.ctx().request_repaint();
            }
        }

        let bordure = if self.deplacement {
            egui::Stroke::new(3.0, JAUNE)
        } else {
            egui::Stroke::NONE
        };
        let tout = ui.max_rect();
        let mut deplacer = None;
        let mut basculer_historique = false;
        let cadre = egui::Frame::new()
            .fill(Color32::from_rgba_unmultiplied(0, 0, 0, self.opacite_fond))
            .stroke(bordure)
            .corner_radius(10)
            .inner_margin(egui::Margin {
                left: 10,
                right: 10,
                top: 4,
                bottom: 0,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                // La bande du haut : on y attrape l'overlay pour le déplacer.
                deplacer = Some(poignee(ui, self.deplacement || self.glisse.is_some()));
                if self.deplacement {
                    ui.label(
                        RichText::new("Tire un bord pour la taille, F8 pour finir")
                            .color(JAUNE)
                            .size(13.0),
                    );
                }
                boutons(ui, &mut partage);
                ui.add_space(2.0);
                choix_combo(ui, &mut partage);
                ui.add_space(4.0);
                etapes(ui, &partage);
                ui.add_space(4.0);
                message(ui, &partage);
                statistiques(ui, &partage);

                if self.historique_ouvert {
                    ui.separator();
                    // L'historique défile à la molette ; une nouvelle ligne le ramène en haut.
                    let mut defilement = egui::ScrollArea::vertical();
                    // Historique du jeu visible : on recopie ses lignes (ce que le jeu a pris
                    // en compte). Sinon, celles de notre manette.
                    let du_jeu = partage.historique_jeu && partage.ligne_jeu_en_cours.is_some();
                    let total = if du_jeu {
                        partage.lignes_jeu_total
                    } else {
                        partage.lignes_total
                    };
                    if total != self.lignes_vues {
                        self.lignes_vues = total;
                        defilement = defilement.vertical_scroll_offset(0.0);
                    }
                    defilement
                        .scroll_source(
                            egui::scroll_area::ScrollSource::MOUSE_WHEEL
                                | egui::scroll_area::ScrollSource::SCROLL_BAR,
                        )
                        .auto_shrink([false, false])
                        .max_height((ui.available_height() - LANGUETTE).max(40.0))
                        .show(ui, |ui| {
                            if du_jeu {
                                historique_du_jeu(ui, &partage);
                            } else {
                                historique(ui, &partage.lignes, partage.syntaxe);
                            }
                        });
                }
                basculer_historique = languette(ui, self.historique_ouvert);
            });
        if basculer_historique {
            self.historique_ouvert = !self.historique_ouvert;
            if self.historique_ouvert {
                self.attendre_hauteur = Some(self.hauteur_ouverte);
            }
            crate::interface::ecrire_cles(
                FICHIER,
                &[("historique_ouvert", self.historique_ouvert.to_string())],
            );
        }
        // Historique replié : la fenêtre se réduit au cadre (plus de fond vide en dessous).
        // Déplié : elle reprend sa hauteur.
        let hauteur_cadre = (cadre.response.rect.max.y - tout.min.y).ceil() + 2.0;
        ajuster_hauteur(
            ui.ctx(),
            hauteur_cadre,
            (self.historique_ouvert, self.glisse.is_some()),
            (&mut self.hauteur_ouverte, &mut self.attendre_hauteur),
        );

        // On bouge la fenêtre nous-mêmes en suivant la souris à l'écran : le glisser de
        // Windows ne marche pas sur une fenêtre qui ne prend pas le focus (pour le laisser
        // à SF6). La bande du haut déplace ; en mode F8, les bords redimensionnent.
        let ctx = ui.ctx().clone();
        let mut fini = None;
        if let Some(bande) = deplacer {
            fini = fini.or(suivre(&ctx, &mut self.glisse, &bande, Bords::default()));
        }
        if self.deplacement {
            for (k, (rect, bords)) in Bords::zones(tout).into_iter().enumerate() {
                let zone = ui.interact(rect, ui.scope_id().with(("bord", k)), egui::Sense::drag());
                fini = fini.or(suivre(&ctx, &mut self.glisse, &zone, bords));
            }
        }
        if let Some(rect) = fini {
            enregistrer_rect(self.place_a_enregistrer(rect));
        }

        // Redessiner un peu plus tard pour faire disparaître le message à temps.
        if partage.message.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
    }
}

/// Repliée, la fenêtre fait la hauteur du cadre ; dépliée, `hauteur_ouverte`
/// (qui suit les redimensionnements faits avec F8).
fn ajuster_hauteur(
    ctx: &egui::Context,
    hauteur_cadre: f32,
    (ouvert, glisse): (bool, bool),
    (hauteur_ouverte, attendre_hauteur): (&mut f32, &mut Option<f32>),
) {
    let Some(fenetre) = ctx.input(|i| i.viewport().inner_rect) else {
        return;
    };
    let voulue = if ouvert {
        match *attendre_hauteur {
            // On vient de déplier : on remet la hauteur jusqu'à ce qu'elle soit prise.
            Some(h) if (fenetre.height() - h).abs() > 1.0 => h,
            _ => {
                *attendre_hauteur = None;
                *hauteur_ouverte = fenetre.height();
                return;
            }
        }
    } else if glisse {
        return;
    } else {
        hauteur_cadre
    };
    if (fenetre.height() - voulue).abs() > 1.0 {
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
            fenetre.width(),
            voulue,
        )));
        ctx.request_repaint();
    }
}

/// Hauteur de la languette de l'historique, en bas.
const LANGUETTE: f32 = 20.0;

/// La bande du haut, avec sa petite poignée : on la glisse pour déplacer l'overlay.
fn poignee(ui: &mut egui::Ui, active: bool) -> egui::Response {
    let (rect, reponse) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 12.0), egui::Sense::drag());
    let vive = active || reponse.hovered() || reponse.dragged();
    let couleur = if vive {
        JAUNE
    } else {
        Color32::from_white_alpha(60)
    };
    let trait_ = egui::Rect::from_center_size(rect.center(), egui::vec2(46.0, 4.0));
    ui.painter().rect_filled(trait_, 2, couleur);
    reponse.on_hover_text("Glisser pour déplacer l'overlay")
}

/// La languette centrée en bas : ouvre ou ferme l'historique. Vrai si on a cliqué.
fn languette(ui: &mut egui::Ui, ouvert: bool) -> bool {
    let (ligne, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), LANGUETTE),
        egui::Sense::hover(),
    );
    let rect = egui::Rect::from_center_size(ligne.center(), egui::vec2(110.0, LANGUETTE - 2.0));
    let reponse = ui.interact(rect, ui.scope_id().with("languette"), egui::Sense::click());
    let couleur = if reponse.hovered() {
        Color32::WHITE
    } else {
        GRIS
    };
    ui.painter().rect_filled(
        rect,
        egui::CornerRadius {
            nw: 6,
            ne: 6,
            sw: 0,
            se: 0,
        },
        Color32::from_white_alpha(if reponse.hovered() { 40 } else { 20 }),
    );
    // Petit triangle : vers le haut pour fermer, vers le bas pour ouvrir.
    let c = egui::pos2(rect.left() + 14.0, rect.center().y);
    let (a, b, d) = if ouvert {
        (
            c + egui::vec2(-5.0, 3.0),
            c + egui::vec2(5.0, 3.0),
            c + egui::vec2(0.0, -3.0),
        )
    } else {
        (
            c + egui::vec2(-5.0, -3.0),
            c + egui::vec2(5.0, -3.0),
            c + egui::vec2(0.0, 3.0),
        )
    };
    ui.painter().add(egui::Shape::convex_polygon(
        vec![a, b, d],
        couleur,
        egui::Stroke::NONE,
    ));
    ui.painter().text(
        egui::pos2(rect.left() + 24.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        "Historique",
        egui::FontId::proportional(13.0),
        couleur,
    );
    reponse.clicked()
}

/// Un glisser sur `zone` : déplace (aucun bord) ou redimensionne la fenêtre.
/// Renvoie la nouvelle place de la fenêtre quand le glisser se termine.
fn suivre(
    ctx: &egui::Context,
    glisse: &mut Option<Glisse>,
    zone: &egui::Response,
    bords: Bords,
) -> Option<egui::Rect> {
    let fenetre = ctx.input(|i| i.viewport().outer_rect)?;
    let a_moi = glisse.as_ref().is_some_and(|g| g.bords == bords);
    if zone.hovered() || a_moi {
        ctx.set_cursor_icon(bords.curseur());
    }
    if zone.drag_started()
        && let Some(curseur) = curseur_ecran(ctx)
    {
        *glisse = Some(Glisse {
            curseur,
            fenetre,
            bords,
            place: fenetre,
        });
        return None;
    }
    if !a_moi {
        return None;
    }
    let g = glisse.as_mut()?;
    if zone.dragged() {
        if let Some(curseur) = curseur_ecran(ctx) {
            let r = g.bords.appliquer(g.fenetre, curseur - g.curseur);
            if r.min != fenetre.min {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(r.min));
            }
            if r.size() != fenetre.size() {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(r.size()));
            }
            g.place = r;
            ctx.request_repaint();
        }
        None
    } else {
        let place = g.place;
        *glisse = None;
        Some(place)
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

/// Les boutons cliquables : son, vidéos, déplacement, photo (et REC à droite).
fn boutons(ui: &mut egui::Ui, p: &mut Partage) {
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

        if p.enregistrement {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // De droite à gauche : le texte, puis le point rouge devant.
                ui.label(RichText::new("REC").color(ROUGE).size(13.0).strong());
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 4.0, ROUGE);
            });
        }
    });
}

/// Le combo en cours, avec ‹ › pour passer au précédent ou au suivant de combos.toml.
fn choix_combo(ui: &mut egui::Ui, p: &mut Partage) {
    let icone = |octets: &'static [u8], nom: &'static str| {
        egui::Image::from_bytes(nom, octets).fit_to_exact_size(egui::vec2(18.0, 18.0))
    };
    let total = p.combos.len();
    ui.horizontal(|ui| {
        if total > 1 {
            let image = icone(
                include_bytes!("../assets/icones/chevron-left.svg"),
                "bytes://precedent.svg",
            );
            if ui
                .add(egui::Button::image(image.tint(Color32::WHITE)).frame(false))
                .on_hover_text("Combo précédent")
                .clicked()
            {
                p.combo_demande = Some((p.combo_index + total - 1) % total);
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if total > 1 {
                let image = icone(
                    include_bytes!("../assets/icones/chevron-right.svg"),
                    "bytes://suivant.svg",
                );
                if ui
                    .add(egui::Button::image(image.tint(Color32::WHITE)).frame(false))
                    .on_hover_text("Combo suivant")
                    .clicked()
                {
                    p.combo_demande = Some((p.combo_index + 1) % total);
                }
                ui.label(
                    RichText::new(format!("{}/{total}", p.combo_index + 1))
                        .color(GRIS)
                        .size(12.0),
                );
            }
            // Le nom prend la place qui reste, coupé avec « … » s'il est trop long.
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(&p.nom)
                            .color(Color32::WHITE)
                            .size(16.0)
                            .strong(),
                    )
                    .truncate(),
                )
                .on_hover_text(&p.nom);
            });
        });
    });
}

/// En bas : les résultats (hits en direct, réussites) puis l'état de la lecture du jeu.
/// Passe à la ligne si l'overlay est étroit.
fn statistiques(ui: &mut egui::Ui, p: &Partage) {
    ui.horizontal_wrapped(|ui| {
        let chiffre = |ui: &mut egui::Ui, n: u32, mot: &str, couleur: Color32| {
            ui.label(
                RichText::new(n.to_string())
                    .color(couleur)
                    .size(16.0)
                    .strong(),
            );
            ui.label(RichText::new(mot).color(GRIS).size(13.0));
            ui.add_space(8.0);
        };
        // Sans hits attendus, le combo ne peut pas être validé.
        if p.hits_attendus.is_none() {
            ui.label(
                RichText::new("combo incomplet (hits inconnus)")
                    .color(JAUNE)
                    .size(13.0)
                    .strong(),
            )
            .on_hover_text(
                "Réussis-le une fois en jeu puis Select, ou saisis ses hits dans l'interface",
            );
            ui.add_space(8.0);
        }
        // Compteur de hits en direct, lu sur le panneau des dégâts du jeu.
        if p.degats_visibles {
            chiffre(ui, p.hits_direct, "hits", ROUGE);
        }
        chiffre(ui, p.reussites_combo, "réussi(s)", VERT);
        if p.serie > 1 {
            chiffre(ui, p.serie, "d'affilée", VERT);
        }
        // Auto-advance : où on en est avant le combo suivant.
        if p.avance_prevue.is_some() {
            ui.label(
                RichText::new("combo suivant…")
                    .color(CYAN)
                    .size(13.0)
                    .strong(),
            );
            ui.add_space(8.0);
        } else if p.entrainement.actif && !p.entrainement.combos.is_empty() {
            let e = &p.entrainement;
            let mot = match e.mode {
                crate::entrainement::Mode::Affilee => "d'affilée",
                crate::entrainement::Mode::Total => "réussis",
            };
            ui.label(
                RichText::new(format!(
                    "auto {}/{} {mot}",
                    e.compte(p.reussites_combo, p.serie).min(e.nombre),
                    e.nombre
                ))
                .color(CYAN)
                .size(13.0),
            );
            ui.add_space(8.0);
        }
        // D'où viennent les frames : l'historique du jeu (exact) ou notre estimation.
        let (texte, couleur) = if p.historique_jeu {
            ("frames du jeu".to_string(), VERT)
        } else {
            match &p.raison_estimee {
                Some(raison) => (format!("frames estimées : {raison}"), JAUNE),
                None => ("frames estimées".to_string(), JAUNE),
            }
        };
        let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 3.0, couleur);
        ui.label(
            RichText::new(texte)
                .color(couleur.gamma_multiply(0.85))
                .size(12.0),
        );
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
        let bord = if cassee {
            egui::Stroke::new(1.5, ROUGE)
        } else {
            egui::Stroke::NONE
        };
        egui::Frame::new()
            .fill(fond)
            .stroke(bord)
            .corner_radius(6)
            .inner_margin(egui::Margin::symmetric(6, 2))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    crate::syntaxe::dessiner_liaison(ui, etape.liaison, p.liaisons, 15.0, true);
                    // Le coup à gauche (il passe à la ligne s'il est trop long), l'écart
                    // « +x f » dans sa colonne à droite : ils ne se chevauchent jamais.
                    let place = (ui.available_width() - TIMING).max(60.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(place, 30.0),
                        egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true),
                        |ui| {
                            crate::syntaxe::dessiner(ui, &etape.notation, p.syntaxe, 24.0, couleur);
                        },
                    );
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), 30.0),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| timing(ui, p, i),
                    );
                });
            });
    }
}

/// Largeur de la colonne de l'écart « +x f », à droite des étapes.
const TIMING: f32 = 58.0;

/// À droite d'une étape : son écart avec la précédente (« +14 f »), pour information.
fn timing(ui: &mut egui::Ui, p: &Partage, i: usize) {
    if i == 0 {
        return;
    }
    let ecart = p.ecarts.get(i - 1).copied();
    {
        if let Some(ecart) = ecart {
            // « ~ » : écart estimé, pas encore vérifié sur l'historique du jeu.
            let approche = if p.ecarts_exacts { "" } else { "~" };
            ui.label(
                RichText::new(format!("{approche}+{ecart} f"))
                    .color(CYAN)
                    .size(15.0)
                    .monospace(),
            );
        }
    }
}

/// « COMBO RÉUSSI » ou « attendu X, reçu Y », pendant 2 secondes.
fn message(ui: &mut egui::Ui, p: &Partage) {
    match &p.message {
        // Un raté reste affiché jusqu'au prochain essai : on a le temps de lire pourquoi.
        Some(m) if !m.reussi || m.quand.elapsed() < DUREE_MESSAGE => {
            let couleur = if m.reussi {
                VERT
            } else if m.attention {
                JAUNE
            } else {
                ROUGE
            };
            match &m.rate {
                // « Raté : attendu X, reçu Y », les coups écrits dans la syntaxe choisie.
                Some((attendu, recu)) => {
                    ui.horizontal_wrapped(|ui| {
                        let mot = |ui: &mut egui::Ui, t: &str| {
                            ui.label(RichText::new(t).color(couleur).size(20.0).strong());
                        };
                        mot(ui, "Raté : attendu");
                        crate::syntaxe::dessiner(ui, attendu, p.syntaxe, 22.0, Color32::WHITE);
                        mot(ui, ", reçu");
                        crate::syntaxe::dessiner(ui, recu, p.syntaxe, 22.0, Color32::WHITE);
                    });
                }
                None => {
                    let taille = if m.attention { 17.0 } else { 22.0 };
                    ui.label(RichText::new(&m.texte).color(couleur).size(taille).strong());
                }
            }
        }
        // Ligne vide de la même hauteur, pour que rien ne saute quand le message apparaît.
        _ => {
            ui.label(RichText::new(" ").size(22.0));
        }
    }
}

/// L'historique lu dans le jeu : la ligne en cours (avec le compteur affiché par le jeu),
/// puis les lignes finies.
fn historique_du_jeu(ui: &mut egui::Ui, p: &Partage) {
    if let Some(en_cours) = &p.ligne_jeu_en_cours {
        let mut ligne = en_cours.clone();
        // Le compteur du jeu, s'il est lu sur la même ligne.
        ligne.frames = crate::ecran::ligne_du_haut()
            .filter(|h| h.direction == ligne.direction && h.boutons == en_cours_bits(&ligne))
            .map_or(1, |h| u64::from(h.frames));
        ligne_encadree(ui, &ligne, p.syntaxe);
    }
    for ligne in &p.lignes_jeu {
        ligne_encadree(ui, ligne, p.syntaxe);
    }
}

/// Les boutons d'une ligne, en bits comme dans l'historique du jeu.
fn en_cours_bits(ligne: &Ligne) -> u8 {
    ligne.boutons.iter().fold(0, |b, (a, _)| {
        b | crate::ecran::boutons_etape(&a.to_string())
    })
}

/// L'historique d'inputs, le plus récent en haut comme dans SF6.
fn historique(ui: &mut egui::Ui, lignes: &std::collections::VecDeque<Ligne>, syntaxe: Syntaxe) {
    for ligne in lignes {
        ligne_encadree(ui, ligne, syntaxe);
    }
}

/// Une ligne de l'historique, encadrée en rouge si le combo a cassé dessus.
fn ligne_encadree(ui: &mut egui::Ui, ligne: &Ligne, syntaxe: Syntaxe) {
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
        .show(ui, |ui| ligne_historique(ui, ligne, syntaxe));
}

/// Une ligne de l'historique : durée, direction, boutons et coups reconnus.
fn ligne_historique(ui: &mut egui::Ui, ligne: &Ligne, syntaxe: Syntaxe) {
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
            if syntaxe == Syntaxe::Sf6 && crate::syntaxe::pastille_action(ui, action, 16.0, nouveau)
            {
                continue;
            }
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
            ui.label(RichText::new("»").color(JAUNE).size(16.0));
            crate::syntaxe::dessiner(ui, coup, syntaxe, 16.0, JAUNE);
        }
        if let Some(etape) = ligne.etape {
            ui.label(
                RichText::new(format!("étape {etape}"))
                    .color(VERT)
                    .size(14.0)
                    .strong(),
            );
        }
        if ligne.cassee {
            ui.label(RichText::new("× break").color(ROUGE).size(14.0).strong());
        }
    });
}

/// Une flèche dans la direction numpad (un point pour le neutre).
pub(crate) fn dessiner_direction(ui: &mut egui::Ui, direction: u8, couleur: Color32) {
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

/// Windows : rend le contenu visible malgré « les clics passent au travers ».
///
/// Pour laisser passer les clics, la fenêtre reçoit le style « layered » de Windows.
/// Une fenêtre layered n'affiche rien tant qu'on ne lui donne pas d'opacité :
/// c'est ce que fait `SetLayeredWindowAttributes`. Bonus : on règle ainsi la
/// transparence de toute la fenêtre.
///
/// `unsafe` : on appelle directement des fonctions C de Windows, Rust ne peut pas
/// vérifier lui-même qu'on les utilise correctement.
/// Écrit la place de la fenêtre dans `overlay.toml`.
fn enregistrer_rect(rect: egui::Rect) {
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
fn regler_fenetre_windows(opacite: u8, traversant: bool) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GWL_EXSTYLE, GetWindowLongPtrW, LWA_ALPHA, SetLayeredWindowAttributes,
        SetWindowLongPtrW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TRANSPARENT,
    };

    // La fenêtre de l'overlay, retrouvée par son titre.
    let titre: Vec<u16> = TITRE.encode_utf16().chain([0]).collect();
    let hwnd = unsafe { FindWindowW(std::ptr::null(), titre.as_ptr()) };
    if hwnd.is_null() {
        return;
    }
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
pub(crate) fn couleur_bouton(action: Action) -> Color32 {
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
