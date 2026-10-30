//! La fenêtre principale de l'outil (hors overlay) : choix du combo, vidéos des combos
//! réussis (image par image, avec les étapes et l'historique du jeu), et réglages.
//!
//! C'est elle qui ouvre l'overlay, dans une deuxième fenêtre (transparente, au premier
//! plan, les clics passent au travers). On peut le cacher ou le réafficher d'ici.

use crate::capture::InfosClip;
use crate::combo::ComboFichier;
use crate::ecran::EntreeJeu;
use crate::overlay::{self, CYAN, GRIS, JAUNE, Overlay, ROUGE, VERT};
use crate::partage::Partage;
use crate::syntaxe::{self, StyleLiaison, Syntaxe};
use crate::video::Video;
use eframe::egui::{self, Color32, RichText};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Une frame du jeu, en unités de 100 ns.
const FRAME: i64 = 166_667;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Onglet {
    Combos,
    Reglages,
}

/// Une vidéo de combo réussi, avec son `.json` s'il existe.
struct Clip {
    video: PathBuf,
    infos: Option<InfosClip>,
}

impl Clip {
    fn titre(&self) -> String {
        match &self.infos {
            Some(i) => format!("{}  —  {}", i.date, i.combo),
            None => self
                .video
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        }
    }
}

/// La vidéo ouverte dans le lecteur.
struct Lecteur {
    video: Video,
    infos: Option<InfosClip>,
    /// Image affichée.
    image: usize,
    lecture: bool,
    vitesse: f32,
    /// Position de lecture (100 ns depuis le début), avancée pendant la lecture.
    position: f64,
    derniere_image: Instant,
    texture: Option<(usize, egui::TextureHandle)>,
    /// Bouton « plein écran » cliqué (l'interface bascule).
    basculer_plein_ecran: bool,
}

pub struct Interface {
    partage: Arc<Mutex<Partage>>,
    overlay: Arc<Mutex<Overlay>>,
    fenetre_overlay: egui::ViewportBuilder,
    overlay_ouvert: bool,
    combos: Vec<ComboFichier>,
    onglet: Onglet,
    recherche: String,
    /// Combo choisi dans la liste (pas forcément celui en cours dans l'overlay).
    choisi: usize,
    dossier_clips: PathBuf,
    /// Images filmées avant la 1re étape (pour les vieux clips sans heures).
    images_avant: usize,
    clips: Vec<Clip>,
    clips_lus: bool,
    /// Le lecteur vidéo occupe tout l'écran.
    plein_ecran: bool,
    lecteur: Option<Lecteur>,
    /// Titre de la fenêtre de SF6 (`fenetre` dans capture.toml).
    titre_jeu: String,
    /// L'overlay est montré (pas caché parce que SF6 n'est pas au premier plan).
    overlay_montre: bool,
}

impl Interface {
    pub fn new(
        partage: Arc<Mutex<Partage>>,
        overlay: Overlay,
        config_overlay: &overlay::Config,
        combos: Vec<ComboFichier>,
        (dossier_clips, avant_ms): (&str, u64),
        titre_jeu: &str,
    ) -> Interface {
        let choisi = partage.lock().map_or(0, |p| p.combo_index);
        Interface {
            partage,
            overlay: Arc::new(Mutex::new(overlay)),
            fenetre_overlay: overlay::fenetre(config_overlay),
            overlay_ouvert: false,
            combos,
            onglet: Onglet::Combos,
            recherche: String::new(),
            choisi,
            dossier_clips: PathBuf::from(dossier_clips),
            images_avant: (avant_ms * 60 / 1000) as usize,
            clips: Vec::new(),
            clips_lus: false,
            plein_ecran: false,
            lecteur: None,
            titre_jeu: titre_jeu.to_string(),
            overlay_montre: true,
        }
    }

    /// Affiche (ou cache) la fenêtre de l'overlay.
    fn fenetre_overlay(&mut self, ctx: &egui::Context, visible: bool) {
        if visible {
            if !self.overlay_ouvert {
                // Une nouvelle fenêtre est créée visible.
                self.overlay_montre = true;
                if let Ok(mut o) = self.overlay.lock() {
                    o.fenetre_recreee();
                }
            }
            let overlay = Arc::clone(&self.overlay);
            ctx.show_viewport_deferred(
                overlay::id(),
                self.fenetre_overlay.clone(),
                move |ui, _| {
                    if let Ok(mut o) = overlay.lock() {
                        o.dessiner(ui);
                    }
                },
            );
        } else if self.overlay_ouvert {
            // Caché : on retient où il était, pour le rouvrir au même endroit.
            if let Some(rect) = ctx.input_for(overlay::id(), |i| i.viewport().outer_rect) {
                self.fenetre_overlay = self
                    .fenetre_overlay
                    .clone()
                    .with_position(rect.min)
                    .with_inner_size(rect.size());
            }
        }
        self.overlay_ouvert = visible;
    }

    /// Relit le dossier des vidéos.
    fn lire_clips(&mut self) {
        self.clips_lus = true;
        self.clips.clear();
        let Ok(entrees) = std::fs::read_dir(&self.dossier_clips) else {
            return;
        };
        for entree in entrees.flatten() {
            let chemin = entree.path();
            let nom = chemin
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if chemin.extension().is_none_or(|e| e != "mp4") || nom.starts_with("en_cours") {
                continue;
            }
            let infos = std::fs::read_to_string(chemin.with_extension("json"))
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok());
            self.clips.push(Clip {
                video: chemin,
                infos,
            });
        }
        // Les plus récentes d'abord (le nom commence par la date).
        self.clips.sort_by(|a, b| b.video.cmp(&a.video));
    }

    fn syntaxe(&self) -> Syntaxe {
        self.partage
            .lock()
            .map_or(Syntaxe::default(), |p| p.syntaxe)
    }
}

impl eframe::App for Interface {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // Le même fond pour les deux fenêtres : celui de l'overlay (transparent ou non).
        // L'interface peint son propre fond par-dessus.
        self.overlay
            .lock()
            .map_or([0.06, 0.06, 0.06, 1.0], |o| o.fond())
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let (visible, jeu_exige) = self.partage.lock().map_or((true, false), |p| {
            (p.overlay_visible, p.seulement_jeu && !p.deplacement)
        });
        self.fenetre_overlay(&ctx, visible);
        // Caché (sans fermer la fenêtre) quand on n'est pas dans SF6.
        let montrer = !jeu_exige || jeu_au_premier_plan(&self.titre_jeu);
        if visible && montrer != self.overlay_montre {
            ctx.send_viewport_cmd_to(overlay::id(), egui::ViewportCommand::Visible(montrer));
            self.overlay_montre = montrer;
        }
        if !self.clips_lus {
            self.lire_clips();
        }

        // Plein écran : seulement le lecteur (Échap pour sortir).
        if self.plein_ecran {
            if self.lecteur.is_none() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.basculer_plein_ecran(&ctx);
            } else {
                let reglages = self.reglages_lecteur();
                let images_avant = self.images_avant;
                egui::CentralPanel::default().show(ui, |ui| {
                    if let Some(lecteur) = &mut self.lecteur {
                        lecteur.afficher(ui, images_avant, reglages, true);
                    }
                });
                if self
                    .lecteur
                    .as_ref()
                    .is_some_and(|l| l.basculer_plein_ecran)
                {
                    self.basculer_plein_ecran(&ctx);
                }
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
                return;
            }
        }

        egui::Panel::top("onglets").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.heading("SF6 Trainer");
                ui.add_space(20.0);
                for (onglet, nom) in [(Onglet::Combos, "Combos"), (Onglet::Reglages, "Réglages")] {
                    if ui
                        .selectable_label(self.onglet == onglet, RichText::new(nom).size(16.0))
                        .clicked()
                    {
                        self.onglet = onglet;
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Ok(mut p) = self.partage.lock() {
                        let texte = if p.overlay_visible {
                            "Cacher l'overlay"
                        } else {
                            "Afficher l'overlay"
                        };
                        if ui.button(texte).clicked() {
                            p.overlay_visible = !p.overlay_visible;
                        }
                        ui.label(
                            RichText::new(format!("En cours : {}", p.nom))
                                .color(JAUNE)
                                .strong(),
                        );
                    }
                });
            });
            ui.add_space(4.0);
        });

        egui::CentralPanel::default().show(ui, |ui| match self.onglet {
            Onglet::Combos => self.onglet_combos(ui),
            Onglet::Reglages => {
                egui::ScrollArea::vertical().show(ui, |ui| self.onglet_reglages(ui));
            }
        });

        // L'état du jeu (historique lu, etc.) change sans clic : on se redessine de temps en temps.
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
    }
}

// ---------------------------------------------------------------- Combos

impl Interface {
    fn onglet_combos(&mut self, ui: &mut egui::Ui) {
        let en_cours = self.partage.lock().map_or(usize::MAX, |p| p.combo_index);
        egui::Panel::left("liste_combos")
            .default_size(340.0)
            .show(ui, |ui| {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("Rechercher");
                    ui.text_edit_singleline(&mut self.recherche);
                });
                ui.add_space(6.0);
                let recherche = self.recherche.to_lowercase();
                // Les catégories, dans l'ordre du fichier.
                let mut categories: Vec<&str> = Vec::new();
                for c in &self.combos {
                    if !categories.contains(&c.categorie.as_str()) {
                        categories.push(&c.categorie);
                    }
                }
                let mut clic = None;
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for categorie in categories {
                        let titre = if categorie.is_empty() {
                            "Combos"
                        } else {
                            categorie
                        };
                        egui::CollapsingHeader::new(RichText::new(titre).strong())
                            .default_open(true)
                            .show(ui, |ui| {
                                for (i, c) in self.combos.iter().enumerate() {
                                    let trouve = recherche.is_empty()
                                        || c.nom.to_lowercase().contains(&recherche)
                                        || c.inputs.to_lowercase().contains(&recherche);
                                    if c.categorie != categorie || !trouve {
                                        continue;
                                    }
                                    let mut texte = RichText::new(&c.nom);
                                    if i == en_cours {
                                        texte = texte.color(JAUNE);
                                    }
                                    if ui.selectable_label(i == self.choisi, texte).clicked() {
                                        clic = Some(i);
                                    }
                                }
                            });
                    }
                });
                if let Some(i) = clic {
                    self.choisi = i;
                }
            });

        let syntaxe = self.syntaxe();
        let liaisons = self
            .partage
            .lock()
            .map_or(Default::default(), |p| p.liaisons);
        egui::CentralPanel::default().show(ui, |ui| {
            let Some(combo) = self.combos.get(self.choisi).cloned() else {
                ui.label("Aucun combo dans combos.toml.");
                return;
            };
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.heading(&combo.nom);
                if !combo.categorie.is_empty() {
                    ui.label(RichText::new(&combo.categorie).color(GRIS));
                }
            });
            ui.label(RichText::new(&combo.inputs).monospace().color(GRIS));
            ui.add_space(6.0);
            match crate::combo::parser(&combo.inputs) {
                Ok(etapes) => {
                    ui.horizontal_wrapped(|ui| {
                        for (i, e) in etapes.iter().enumerate() {
                            if i > 0 {
                                syntaxe::dessiner_liaison(ui, e.liaison, liaisons, 15.0, false);
                            }
                            syntaxe::dessiner(ui, &e.notation, syntaxe, 20.0, Color32::WHITE);
                        }
                    });
                }
                Err(e) => {
                    ui.label(RichText::new(e).color(ROUGE));
                }
            }
            ui.add_space(12.0);

            // Ce qui valide le combo : du fichier, sinon appris avec Select.
            let appris = crate::timings::charger().ok();
            let hits = combo
                .hits
                .map(|h| format!("{h}"))
                .or_else(|| {
                    appris
                        .as_ref()
                        .and_then(|t| t.hits(&combo.nom))
                        .map(|h| format!("{h} (appris)"))
                })
                .unwrap_or_else(|| "pas encore appris (Select après un vrai combo)".into());
            let degats = combo
                .degats
                .map(|d| format!("{d}"))
                .or_else(|| {
                    appris
                        .as_ref()
                        .and_then(|t| t.degats(&combo.nom))
                        .map(|d| format!("{d} (appris)"))
                })
                .unwrap_or_else(|| "pas encore appris".into());
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Hits").color(GRIS));
                ui.label(RichText::new(hits).strong());
                ui.add_space(12.0);
                ui.label(RichText::new("Dégâts").color(GRIS));
                ui.label(RichText::new(degats).strong());
                ui.add_space(20.0);
                let lance = en_cours == self.choisi;
                let bouton = egui::Button::new(
                    RichText::new(if lance {
                        "Combo en cours"
                    } else {
                        "Travailler ce combo"
                    })
                    .size(16.0),
                );
                if ui.add_enabled(!lance, bouton).clicked()
                    && let Ok(mut p) = self.partage.lock()
                {
                    p.combo_demande = Some(self.choisi);
                }
            });
            ui.add_space(6.0);
            ui.separator();
            self.videos_du_combo(ui, &combo.nom);
        });
    }
}

// ---------------------------------------------------------------- Vidéos

impl Interface {
    /// Syntaxe et liaisons choisies, pour le lecteur.
    fn reglages_lecteur(&self) -> (Syntaxe, StyleLiaison) {
        self.partage
            .lock()
            .map_or(Default::default(), |p| (p.syntaxe, p.liaisons))
    }

    fn ouvrir_clip(&mut self, i: usize) {
        let Some(clip) = self.clips.get(i) else {
            return;
        };
        self.lecteur = Some(Lecteur {
            video: Video::ouvrir(&clip.video),
            infos: clip.infos.clone(),
            image: 0,
            lecture: false,
            vitesse: 1.0,
            position: 0.0,
            derniere_image: Instant::now(),
            texture: None,
            basculer_plein_ecran: false,
        });
    }

    fn basculer_plein_ecran(&mut self, ctx: &egui::Context) {
        self.plein_ecran = !self.plein_ecran;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.plein_ecran));
        if let Some(l) = &mut self.lecteur {
            l.basculer_plein_ecran = false;
        }
    }

    /// Sous le combo : ses vidéos (la plus récente ouverte d'office) et le lecteur.
    fn videos_du_combo(&mut self, ui: &mut egui::Ui, nom: &str) {
        let siens: Vec<usize> = (0..self.clips.len())
            .filter(|&i| self.clips[i].infos.as_ref().is_some_and(|c| c.combo == nom))
            .collect();
        // Le lecteur suit le combo choisi.
        let lecteur_du_combo = self
            .lecteur
            .as_ref()
            .is_some_and(|l| l.infos.as_ref().is_some_and(|c| c.combo == nom));
        if !lecteur_du_combo {
            self.lecteur = None;
            if let Some(&i) = siens.first() {
                self.ouvrir_clip(i);
            }
        }

        ui.horizontal(|ui| {
            ui.heading(format!("Vidéos ({})", siens.len()));
            if ui.small_button("Actualiser").clicked() {
                self.lire_clips();
            }
            if siens.len() > 1 {
                let ouvert = self.lecteur.as_ref().map(|l| l.video.chemin.clone());
                let titre = |i: usize| {
                    self.clips[i]
                        .infos
                        .as_ref()
                        .map_or_else(|| self.clips[i].titre(), |c| c.date.clone())
                };
                let actuel = siens
                    .iter()
                    .find(|&&i| Some(&self.clips[i].video) == ouvert.as_ref())
                    .map_or_else(String::new, |&i| titre(i));
                let mut choix = None;
                egui::ComboBox::from_id_salt("clip")
                    .selected_text(actuel)
                    .show_ui(ui, |ui| {
                        for &i in &siens {
                            let pris = Some(&self.clips[i].video) == ouvert.as_ref();
                            if ui.selectable_label(pris, titre(i)).clicked() {
                                choix = Some(i);
                            }
                        }
                    });
                if let Some(i) = choix {
                    self.ouvrir_clip(i);
                }
            }
        });
        if siens.is_empty() {
            ui.label(
                RichText::new(
                    "Pas encore de vidéo pour ce combo. Active les vidéos (Réglages ou F9) : \
                     chaque combo réussi est filmé.",
                )
                .color(GRIS),
            );
            return;
        }
        ui.add_space(4.0);
        let reglages = self.reglages_lecteur();
        let images_avant = self.images_avant;
        if let Some(lecteur) = &mut self.lecteur {
            lecteur.afficher(ui, images_avant, reglages, false);
        }
        if self
            .lecteur
            .as_ref()
            .is_some_and(|l| l.basculer_plein_ecran)
        {
            self.basculer_plein_ecran(ui.ctx());
        }
    }
}

impl Lecteur {
    fn afficher(
        &mut self,
        ui: &mut egui::Ui,
        images_avant: usize,
        (syntaxe, liaisons): (Syntaxe, StyleLiaison),
        plein_ecran: bool,
    ) {
        let Ok(decodage) = self.video.decodage.lock() else {
            return;
        };
        if let Some(e) = &decodage.erreur {
            ui.label(RichText::new(format!("Vidéo illisible : {e}")).color(ROUGE));
            return;
        }
        let total = decodage.images.len();
        if total == 0 {
            ui.label("Décodage de la vidéo…");
            ui.ctx().request_repaint();
            return;
        }
        if !decodage.fini {
            ui.ctx().request_repaint();
        }
        let temps: Vec<i64> = decodage.images.iter().map(|i| i.temps).collect();

        // Lecture : on avance selon le temps écoulé.
        let maintenant = Instant::now();
        let ecoule = maintenant.duration_since(self.derniere_image).as_secs_f64();
        self.derniere_image = maintenant;
        if self.lecture {
            self.position += ecoule * 10_000_000.0 * f64::from(self.vitesse);
            let i = temps.partition_point(|&t| (t as f64) <= self.position);
            self.image = i.saturating_sub(1);
            if self.image + 1 >= total && decodage.fini {
                self.lecture = false;
            }
            ui.ctx().request_repaint();
        }

        // Clavier : ← → image par image, espace = lecture / pause.
        let (gauche, droite, espace) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
                i.key_pressed(egui::Key::Space),
            )
        });
        let mut aller = None;
        if gauche {
            aller = Some(self.image.saturating_sub(1));
        }
        if droite {
            aller = Some(self.image + 1);
        }
        if espace {
            self.lecture = !self.lecture;
        }

        // Où sont les étapes dans la vidéo, et l'heure absolue de chaque image.
        let etapes = self.etapes_dans_la_video(&temps, images_avant);
        let debut = self.infos.as_ref().map_or(0, |i| i.debut_video);

        // L'image.
        self.image = self.image.min(total - 1);
        let image = &decodage.images[self.image];
        let texture = match &self.texture {
            Some((n, t)) if *n == self.image => t.clone(),
            _ => {
                let couleurs = egui::ColorImage::from_rgb(
                    [decodage.largeur as usize, decodage.hauteur as usize],
                    &image.rgb,
                );
                let t = ui
                    .ctx()
                    .load_texture("video", couleurs, egui::TextureOptions::LINEAR);
                self.texture = Some((self.image, t.clone()));
                t
            }
        };
        let temps_image = image.temps;
        drop(decodage);

        let historique_largeur = 230.0;
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                let largeur = (ui.available_width() - historique_largeur - 10.0).max(200.0);
                let taille = texture.size_vec2();
                // Assez de place en dessous pour la barre, les boutons et les étapes.
                let hauteur = (ui.available_height() - 130.0).max(120.0);
                let echelle = (largeur / taille.x).min(hauteur / taille.y);
                ui.add(egui::Image::new(&texture).fit_to_exact_size(taille * echelle));

                // Barre de temps avec les étapes.
                let mut image = self.image;
                let barre = ui.add(
                    egui::Slider::new(&mut image, 0..=total - 1)
                        .show_value(false)
                        .trailing_fill(true),
                );
                if barre.changed() {
                    aller = Some(image);
                }
                let rect = barre.rect;
                let peintre = ui.painter();
                for &(i, ref notation) in &etapes {
                    let x = rect.left() + rect.width() * i as f32 / (total.max(2) - 1) as f32;
                    let couleur = if i <= self.image { VERT } else { JAUNE };
                    peintre.vline(x, rect.y_range(), egui::Stroke::new(2.0, couleur));
                    peintre.text(
                        egui::pos2(x, rect.top() - 2.0),
                        egui::Align2::CENTER_BOTTOM,
                        syntaxe::texte_simple(notation, syntaxe),
                        egui::FontId::proportional(12.0),
                        couleur,
                    );
                }

                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("|<").on_hover_text("Début").clicked() {
                        aller = Some(0);
                    }
                    if ui
                        .button("< 1")
                        .on_hover_text("Image précédente (flèche gauche)")
                        .clicked()
                    {
                        aller = Some(self.image.saturating_sub(1));
                    }
                    let texte = if self.lecture { "Pause" } else { "Lecture" };
                    if ui.button(texte).on_hover_text("Espace").clicked() {
                        self.lecture = !self.lecture;
                    }
                    if ui
                        .button("1 >")
                        .on_hover_text("Image suivante (flèche droite)")
                        .clicked()
                    {
                        aller = Some(self.image + 1);
                    }
                    ui.add_space(10.0);
                    for v in [1.0, 0.5, 0.25, 0.1] {
                        if ui
                            .selectable_label(self.vitesse == v, format!("x{v}"))
                            .clicked()
                        {
                            self.vitesse = v;
                        }
                    }
                    ui.add_space(10.0);
                    let texte = if plein_ecran {
                        "Quitter le plein écran (Échap)"
                    } else {
                        "Plein écran"
                    };
                    if ui.button(texte).clicked() {
                        self.basculer_plein_ecran = true;
                    }
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new(format!(
                            "image {} / {total}   {:.3} s",
                            self.image + 1,
                            temps_image as f64 / 10_000_000.0
                        ))
                        .monospace(),
                    );
                });

                // Les étapes : cliquer dessus y amène.
                ui.add_space(6.0);
                // Les liaisons (cancel / link), relues dans les inputs du combo.
                let liens: Vec<crate::combo::Liaison> = self
                    .infos
                    .as_ref()
                    .and_then(|infos| crate::combo::parser(&infos.inputs).ok())
                    .map(|e| e.iter().map(|e| e.liaison).collect())
                    .unwrap_or_default();
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Étapes :").color(GRIS));
                    for (k, (i, notation)) in etapes.iter().enumerate() {
                        if k > 0
                            && let Some(&liaison) = liens.get(k)
                        {
                            syntaxe::dessiner_liaison(ui, liaison, liaisons, 14.0, false);
                        }
                        let faite = *i <= self.image;
                        let couleur = if faite { VERT } else { Color32::WHITE };
                        if syntaxe::bouton_coup(ui, notation, syntaxe, 16.0, couleur)
                            .on_hover_text(format!("image {}", i + 1))
                            .clicked()
                        {
                            aller = Some(*i);
                        }
                    }
                });
                if etapes.is_empty() {
                    ui.label(
                        RichText::new("Pas d'étapes enregistrées pour cette vidéo.").color(GRIS),
                    );
                }
            });

            // L'historique du jeu à cet instant de la vidéo.
            ui.vertical(|ui| {
                ui.set_width(historique_largeur);
                ui.label(RichText::new("Historique du jeu").strong());
                match &self.infos {
                    Some(infos) if !infos.lignes.is_empty() && debut != 0 => {
                        historique_a(ui, &infos.lignes, debut + temps_image, syntaxe);
                    }
                    _ => {
                        ui.label(
                            RichText::new(
                                "Pas d'historique pour cette vidéo (vidéos enregistrées \
                                 avant cette version, ou historique du jeu pas affiché).",
                            )
                            .color(GRIS),
                        );
                    }
                }
            });
        });

        if let Some(i) = aller {
            self.lecture = false;
            self.image = i.min(total - 1);
            self.position = temps.get(self.image).copied().unwrap_or(0) as f64;
        }
    }

    /// L'image de chaque étape dans la vidéo (et sa notation).
    fn etapes_dans_la_video(&self, temps: &[i64], images_avant: usize) -> Vec<(usize, String)> {
        let Some(infos) = &self.infos else {
            return Vec::new();
        };
        infos
            .etapes
            .iter()
            .map(|e| {
                let i = if infos.debut_video != 0 && e.temps != 0 {
                    // L'appui apparaît à l'écran (dans l'historique du jeu) après la latence.
                    let t = e.temps + infos.latence - infos.debut_video;
                    temps.partition_point(|&x| x < t)
                } else {
                    // Vieux clips : la vidéo commence `images_avant` images avant la 1re étape.
                    images_avant + e.frame as usize
                };
                (i.min(temps.len().saturating_sub(1)), e.notation.clone())
            })
            .collect()
    }
}

/// L'historique du jeu tel qu'il était affiché à l'heure `maintenant` (le plus récent en haut).
fn historique_a(ui: &mut egui::Ui, lignes: &[EntreeJeu], maintenant: i64, syntaxe: Syntaxe) {
    let visibles: Vec<&EntreeJeu> = lignes.iter().filter(|e| e.vu <= maintenant).collect();
    if visibles.is_empty() {
        ui.label(RichText::new("(rien encore)").color(GRIS));
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt("historique_video")
        .show(ui, |ui| {
            for (k, e) in visibles.iter().enumerate().rev().take(20) {
                // Durée : jusqu'à la ligne suivante, ou en cours (estimée avec l'heure).
                let frames = match visibles.get(k + 1) {
                    Some(suivante) if suivante.segment == e.segment => {
                        suivante.frame.saturating_sub(e.frame)
                    }
                    Some(_) => 99,
                    None => ((maintenant - e.vu) / FRAME + 1).max(1) as u64,
                };
                ui.horizontal(|ui| {
                    let couleur = if k + 1 == visibles.len() { JAUNE } else { CYAN };
                    ui.label(
                        RichText::new(format!("{:>3}", frames.min(99)))
                            .monospace()
                            .color(couleur),
                    );
                    let couleur_dir = if e.direction == 5 {
                        GRIS
                    } else {
                        Color32::WHITE
                    };
                    overlay::dessiner_direction(ui, e.direction, couleur_dir);
                    for (bit, action) in [
                        (crate::ecran::LP, crate::controles::Action::LP),
                        (crate::ecran::MP, crate::controles::Action::MP),
                        (crate::ecran::HP, crate::controles::Action::HP),
                        (crate::ecran::LK, crate::controles::Action::LK),
                        (crate::ecran::MK, crate::controles::Action::MK),
                        (crate::ecran::HK, crate::controles::Action::HK),
                    ] {
                        if e.boutons & bit != 0 {
                            if syntaxe == Syntaxe::Sf6 {
                                syntaxe::pastille_action(ui, action, 14.0, e.nouveaux & bit != 0);
                                continue;
                            }
                            let mut t = RichText::new(action.to_string())
                                .color(overlay::couleur_bouton(action));
                            t = if e.nouveaux & bit != 0 {
                                t.strong()
                            } else {
                                t.weak()
                            };
                            ui.label(t);
                        }
                    }
                });
            }
        });
}

// ---------------------------------------------------------------- Réglages

impl Interface {
    fn onglet_reglages(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        let Ok(mut p) = self.partage.lock() else {
            return;
        };
        ui.heading("Overlay");
        ui.checkbox(
            &mut p.overlay_visible,
            "Afficher l'overlay par-dessus le jeu",
        );
        if ui
            .checkbox(
                &mut p.seulement_jeu,
                "Seulement quand SF6 est la fenêtre active",
            )
            .changed()
        {
            ecrire_cles(
                "overlay.toml",
                &[("seulement_jeu", p.seulement_jeu.to_string())],
            );
        }
        ui.checkbox(
            &mut p.deplacement,
            "Déplacer / redimensionner l'overlay à la souris (F8)",
        );
        if let Ok(mut o) = self.overlay.lock() {
            let mut fond = o.opacite_fond();
            let mut fenetre = o.opacite_fenetre();
            let a = ui.add(egui::Slider::new(&mut fond, 0..=255).text("Opacité du fond"));
            let b = ui.add(egui::Slider::new(&mut fenetre, 30..=255).text("Opacité de l'overlay"));
            o.regler_opacite_fond(fond);
            o.regler_opacite_fenetre(fenetre);
            if a.drag_stopped() || b.drag_stopped() || a.lost_focus() || b.lost_focus() {
                ecrire_cles(
                    "overlay.toml",
                    &[
                        ("opacite_fond", fond.to_string()),
                        ("opacite_fenetre", fenetre.to_string()),
                    ],
                );
            }
        }

        ui.add_space(14.0);
        ui.heading("Écriture des coups");
        ui.label(RichText::new("Pour l'overlay, la liste des combos et les vidéos.").color(GRIS));
        let avant = p.syntaxe;
        ui.horizontal_wrapped(|ui| {
            for s in Syntaxe::TOUTES {
                ui.radio_value(&mut p.syntaxe, s, s.nom());
            }
        });
        let avant_liaisons = p.liaisons;
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Liaisons :").color(GRIS));
            for s in StyleLiaison::TOUS {
                ui.radio_value(&mut p.liaisons, s, s.nom());
            }
        });
        ui.horizontal(|ui| {
            use crate::combo::Liaison;
            ui.label(RichText::new("Exemple :").color(GRIS));
            for (liaison, coup) in [
                (Liaison::Debut, "1HP"),
                (Liaison::Cancel, "DR"),
                (Liaison::Cancel, "1MK"),
                (Liaison::Cancel, "236MK"),
                (Liaison::Link, "1MK"),
                (Liaison::Cancel, "63214MP"),
            ] {
                syntaxe::dessiner_liaison(ui, liaison, p.liaisons, 15.0, false);
                syntaxe::dessiner(ui, coup, p.syntaxe, 20.0, Color32::WHITE);
            }
        });
        if p.syntaxe != avant || p.liaisons != avant_liaisons {
            ecrire_cles(
                "overlay.toml",
                &[
                    ("syntaxe", format!("\"{}\"", p.syntaxe.cle())),
                    ("liaisons", format!("\"{}\"", p.liaisons.cle())),
                ],
            );
        }

        ui.add_space(14.0);
        ui.heading("Son et vidéos");
        let mut son = crate::son::actif();
        if ui
            .checkbox(&mut son, "Son à chaque étape et au combo réussi")
            .changed()
        {
            crate::son::activer(son);
            ecrire_cles("overlay.toml", &[("son", son.to_string())]);
        }
        ui.checkbox(&mut p.enregistrement, "Filmer les combos réussis (F9)");
        ui.horizontal(|ui| {
            ui.label(RichText::new("Dossier des vidéos :").color(GRIS));
            ui.label(self.dossier_clips.display().to_string());
            if ui.button("Ouvrir").clicked() {
                ouvrir_dossier(&self.dossier_clips);
            }
        });

        ui.add_space(14.0);
        ui.heading("Lecture du jeu");
        let etat = |ok: bool, oui: &str, non: &str| {
            if ok {
                RichText::new(format!("• {oui}")).color(VERT)
            } else {
                RichText::new(format!("• {non}")).color(JAUNE)
            }
        };
        ui.label(etat(
            p.historique_jeu,
            "historique d'inputs du jeu lu à l'écran (coups et frames du jeu)",
            "historique d'inputs du jeu pas visible : active-le dans le mode entraînement",
        ));
        ui.label(etat(
            p.degats_visibles,
            "panneau des dégâts lu (hits et dégâts du combo)",
            "panneau des dégâts pas visible : affiche les infos de dégâts du training",
        ));

        ui.add_space(14.0);
        ui.heading("Raccourcis en jeu");
        egui::Grid::new("raccourcis")
            .spacing([20.0, 4.0])
            .show(ui, |ui| {
                for (touche, effet) in [
                    ("F8", "déplacer l'overlay"),
                    ("F9", "filmer ou non les combos réussis"),
                    ("F10", "photo du jeu (PNG)"),
                    (
                        "Select",
                        "« c'était un vrai combo » : apprend ses hits et dégâts",
                    ),
                ] {
                    ui.label(RichText::new(touche).strong());
                    ui.label(effet);
                    ui.end_row();
                }
            });
        ui.add_space(10.0);
        ui.label(
            RichText::new(
                "Autres réglages (capture, manette...) : capture.toml et controles.toml, \
                 dans le dossier de l'outil (relancer après modification).",
            )
            .color(GRIS),
        );
    }
}

/// Remplace la valeur de quelques clés (`cle = valeur`) dans un fichier de réglages,
/// sans toucher au reste (commentaires compris). Une clé absente est ajoutée à la fin.
pub(crate) fn ecrire_cles(fichier: &str, valeurs: &[(&str, String)]) {
    let Ok(texte) = std::fs::read_to_string(fichier) else {
        return;
    };
    let nouveau = remplacer_cles(&texte, valeurs);
    if let Err(e) = std::fs::write(fichier, nouveau) {
        eprintln!("impossible d'écrire {fichier} : {e}");
    }
}

fn remplacer_cles(texte: &str, valeurs: &[(&str, String)]) -> String {
    let mut vues = vec![false; valeurs.len()];
    let mut sortie = String::new();
    for ligne in texte.lines() {
        let cle = ligne.split('=').next().unwrap_or("").trim();
        match valeurs.iter().position(|(nom, _)| {
            *nom == cle && ligne.contains('=') && !ligne.trim_start().starts_with('#')
        }) {
            Some(i) => {
                vues[i] = true;
                sortie.push_str(&format!("{} = {}", valeurs[i].0, valeurs[i].1));
            }
            None => sortie.push_str(ligne),
        }
        sortie.push('\n');
    }
    for (i, (nom, valeur)) in valeurs.iter().enumerate() {
        if !vues[i] {
            sortie.push_str(&format!("{nom} = {valeur}\n"));
        }
    }
    sortie
}

/// Vrai si la fenêtre active est celle de SF6 (son titre est exactement `titre`,
/// comme pour la capture vidéo).
#[cfg(windows)]
fn jeu_au_premier_plan(titre: &str) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW};
    if titre.is_empty() {
        return true;
    }
    let mut tampon = [0u16; 256];
    let longueur = unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return false;
        }
        GetWindowTextW(hwnd, tampon.as_mut_ptr(), tampon.len() as i32)
    };
    let actif = String::from_utf16_lossy(&tampon[..longueur.max(0) as usize]);
    // Titre exact : un onglet de navigateur « Street Fighter 6/Dhalsim » ne compte pas.
    actif.trim() == titre.trim()
}

#[cfg(not(windows))]
fn jeu_au_premier_plan(_titre: &str) -> bool {
    true
}

/// Ouvre un dossier dans l'explorateur de fichiers.
fn ouvrir_dossier(dossier: &Path) {
    let _ = std::fs::create_dir_all(dossier);
    #[cfg(windows)]
    let programme = "explorer";
    #[cfg(not(windows))]
    let programme = "xdg-open";
    let _ = std::process::Command::new(programme).arg(dossier).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remplacer_des_cles_garde_le_reste() {
        let texte = "# commentaire\nson = true\nopacite_fond = 170\n";
        let nouveau = remplacer_cles(
            texte,
            &[("son", "false".into()), ("opacite_fenetre", "200".into())],
        );
        assert_eq!(
            nouveau,
            "# commentaire\nson = false\nopacite_fond = 170\nopacite_fenetre = 200\n"
        );
    }
}
