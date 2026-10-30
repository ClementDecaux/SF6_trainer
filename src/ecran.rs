//! Lecture de l'historique d'inputs que SF6 affiche à gauche de l'écran.
//!
//! Le jeu y écrit, pour chaque état de la manette (direction + boutons), le nombre exact
//! de frames qu'il a duré. Notre horloge ne peut pas faire mieux qu'une estimation :
//! elle ne sait pas à quel moment précis le jeu lit la manette. En lisant l'écran, on a
//! les vraies durées du jeu.
//!
//! Comment : chaque image capturée, on lit les lignes du haut de l'historique (chiffres,
//! flèche, boutons) en les comparant à des modèles pris sur des photos du jeu
//! (`ecran_modeles.rs`). Une ligne mal lue (cachée par « 3 HITS », « PUNISH »...) est
//! ignorée. `Suivi` repère les nouvelles lignes d'une image à l'autre et en tient le journal,
//! avec la frame du jeu où chacune commence.
//!
//! Mise en page mesurée en 1920x1080, mise à l'échelle pour les autres résolutions.

// Hors Windows, pas de capture : une partie du code ne sert qu'aux tests.
#![cfg_attr(not(windows), allow(dead_code))]

use crate::ecran_modeles::{
    BOUTON_C, CHIFFRE_H, CHIFFRE_L, CHIFFRES, CHIFFRES_DEGATS, DEGAT_H, DEGAT_L, DIRECTION_C,
    DIRECTIONS, PIED, POING,
};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, OnceLock};

/// Une frame à 60 images par seconde, en unités de 100 ns (horloge de la capture).
pub const FRAME: i64 = 166_667;

// Positions en 1080p : haut des chiffres de la 1re ligne, puis une ligne tous les 34 px.
const Y0: i32 = 236;
const PAS: i32 = 34;
const DIZAINES_X: i32 = 57;
const UNITES_X: i32 = 68;
const DIRECTION_X: i32 = 93;
const BOUTON_X: i32 = 138;
const BOUTON_PAS: i32 = 30;
/// Lignes lues à chaque image : assez pour retrouver les anciennes après plusieurs
/// nouvelles lignes d'un coup (si la capture a sauté une image).
pub const LIGNES_LUES: usize = 8;

/// Les boutons, un bit chacun.
pub const LP: u8 = 1;
pub const MP: u8 = 2;
pub const HP: u8 = 4;
pub const LK: u8 = 8;
pub const MK: u8 = 16;
pub const HK: u8 = 32;

/// Les bits d'un bouton écrit en texte (« MK »), 0 s'il est inconnu.
pub fn bit(bouton: &str) -> u8 {
    match bouton {
        "LP" => LP,
        "MP" => MP,
        "HP" => HP,
        "LK" => LK,
        "MK" => MK,
        "HK" => HK,
        _ => 0,
    }
}

/// Une ligne de l'historique du jeu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LigneJeu {
    /// Durée en frames (le jeu s'arrête à 99).
    pub frames: u8,
    /// Direction en numpad (5 = neutre).
    pub direction: u8,
    /// Boutons tenus (bits LP, MP...).
    pub boutons: u8,
}

impl LigneJeu {
    fn meme_etat(&self, autre: &LigneJeu) -> bool {
        self.direction == autre.direction && self.boutons == autre.boutons
    }
}

/// Une image du jeu, en pixels BGRA (format de la capture Windows), lignes de haut en bas.
pub struct Vue<'a> {
    pixels: &'a [u8],
    largeur: usize,
    hauteur: usize,
    /// Octets par ligne (souvent un peu plus que largeur × 4).
    pas: usize,
    echelle: f32,
    ox: f32,
    oy: f32,
}

impl<'a> Vue<'a> {
    /// L'image entière du jeu : l'échelle vient de la hauteur (1080 = 1),
    /// et si l'image est plus large que du 16/9, le jeu est centré (bandes noires).
    pub fn jeu(pixels: &'a [u8], largeur: usize, hauteur: usize, pas: usize) -> Vue<'a> {
        let echelle = hauteur as f32 / 1080.0;
        let ox = ((largeur as f32 - 1920.0 * echelle) / 2.0).max(0.0);
        Vue {
            pixels,
            largeur,
            hauteur,
            pas,
            echelle,
            ox,
            oy: 0.0,
        }
    }

    /// La partie de l'image du jeu qui est lue (historique d'inputs et panneau des dégâts),
    /// en pixels de l'image entière : (x0, y0, x1, y1). Copier seulement cette partie depuis
    /// la carte graphique coûte bien moins cher que l'image entière.
    pub fn zone_lue(largeur: u32, hauteur: u32) -> (u32, u32, u32, u32) {
        let echelle = hauteur as f32 / 1080.0;
        let ox = ((largeur as f32 - 1920.0 * echelle) / 2.0).max(0.0);
        let px = |v: f32| v.round().max(0.0) as u32;
        (
            px(ox),
            px(150.0 * echelle).min(hauteur),
            px(ox + 820.0 * echelle).min(largeur),
            px(920.0 * echelle).min(hauteur),
        )
    }

    /// Le morceau `zone_lue` d'une image du jeu de `largeur` × `hauteur` pixels.
    pub fn zone(
        pixels: &'a [u8],
        (largeur_jeu, hauteur_jeu): (u32, u32),
        (largeur, hauteur): (usize, usize),
        pas: usize,
    ) -> Vue<'a> {
        let entiere = Vue::jeu(pixels, largeur_jeu as usize, hauteur_jeu as usize, pas);
        let (x0, y0, _, _) = Vue::zone_lue(largeur_jeu, hauteur_jeu);
        Vue {
            pixels,
            largeur,
            hauteur,
            pas,
            echelle: entiere.echelle,
            ox: entiere.ox - x0 as f32,
            oy: -(y0 as f32),
        }
    }

    #[cfg(test)]
    /// Un morceau d'image en 1080p dont le coin haut gauche est en (`x`, `y`) (pour les tests).
    pub fn morceau(pixels: &'a [u8], largeur: usize, hauteur: usize, x: i32, y: i32) -> Vue<'a> {
        Vue {
            pixels,
            largeur,
            hauteur,
            pas: largeur * 4,
            echelle: 1.0,
            ox: -x as f32,
            oy: -y as f32,
        }
    }

    /// Le pixel (rouge, vert, bleu) aux coordonnées 1080p (`x`, `y`). Noir hors de l'image.
    fn rgb(&self, x: i32, y: i32) -> [u8; 3] {
        let fx = self.ox + (x as f32 + 0.5) * self.echelle;
        let fy = self.oy + (y as f32 + 0.5) * self.echelle;
        if fx < 0.0 || fy < 0.0 {
            return [0; 3];
        }
        let (px, py) = (fx as usize, fy as usize);
        if px >= self.largeur || py >= self.hauteur {
            return [0; 3];
        }
        let i = py * self.pas + px * 4;
        match self.pixels.get(i..i + 3) {
            Some(p) => [p[2], p[1], p[0]],
            None => [0; 3],
        }
    }

    /// Masque des pixels blancs (texte et flèches) d'un rectangle : 1 = blanc.
    fn blancs(&self, x: i32, y: i32, l: usize, h: usize) -> Vec<f32> {
        let mut masque = Vec::with_capacity(l * h);
        for dy in 0..h as i32 {
            for dx in 0..l as i32 {
                let p = self.rgb(x + dx, y + dy);
                masque.push(if p.iter().all(|&c| c > 200) { 1.0 } else { 0.0 });
            }
        }
        masque
    }
}

/// Un modèle prêt à comparer : centré et normé (corrélation normalisée).
struct Modele {
    valeurs: Vec<f32>,
}

impl Modele {
    fn new(octets: &[u8]) -> Modele {
        Modele {
            valeurs: centrer(&octets.iter().map(|&v| v as f32 / 255.0).collect::<Vec<_>>()),
        }
    }

    /// Ressemblance de -1 à 1 (1 = identique) avec un échantillon déjà centré.
    fn score(&self, echantillon: &[f32]) -> f32 {
        self.valeurs
            .iter()
            .zip(echantillon)
            .map(|(a, b)| a * b)
            .sum()
    }
}

/// Retire la moyenne et ramène à une norme de 1 (tout à zéro si l'échantillon est uni).
fn centrer(valeurs: &[f32]) -> Vec<f32> {
    let moyenne = valeurs.iter().sum::<f32>() / valeurs.len() as f32;
    let centre: Vec<f32> = valeurs.iter().map(|v| v - moyenne).collect();
    let norme = centre.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norme < 1e-6 {
        return vec![0.0; valeurs.len()];
    }
    centre.iter().map(|v| v / norme).collect()
}

struct Modeles {
    chiffres: Vec<Modele>,
    directions: Vec<Modele>,
    poing: Modele,
    pied: Modele,
}

fn modeles() -> &'static Modeles {
    static MODELES: OnceLock<Modeles> = OnceLock::new();
    MODELES.get_or_init(|| Modeles {
        chiffres: CHIFFRES.iter().map(|c| Modele::new(c)).collect(),
        directions: DIRECTIONS.iter().map(|d| Modele::new(d)).collect(),
        poing: Modele::new(&POING),
        pied: Modele::new(&PIED),
    })
}

/// Le meilleur modèle : (index, score, écart avec le 2e).
fn meilleur(modeles: &[Modele], echantillon: &[f32]) -> (usize, f32, f32) {
    let echantillon = centrer(echantillon);
    let mut scores: Vec<(usize, f32)> = modeles
        .iter()
        .enumerate()
        .map(|(i, m)| (i, m.score(&echantillon)))
        .collect();
    scores.sort_by(|a, b| b.1.total_cmp(&a.1));
    (scores[0].0, scores[0].1, scores[0].1 - scores[1].1)
}

// Scores minimaux pour croire une lecture (mesurés sur les photos : les bonnes lectures
// font 0,9 à 1, les lignes cachées par un autre affichage tombent sous 0,7).
const SCORE_CHIFFRE: f32 = 0.8;
const SCORE_DIRECTION: f32 = 0.75;
const ECART_DIRECTION: f32 = 0.08;
const SCORE_BOUTON: f32 = 0.75;

/// Lit la ligne `k` de l'historique (0 = la plus récente, en haut).
/// `None` si elle est vide ou douteuse.
pub fn lire_ligne(vue: &Vue, k: usize) -> Option<LigneJeu> {
    let m = modeles();
    let y = Y0 + PAS * k as i32;

    // La durée : unités, puis dizaines s'il y a quelque chose d'écrit.
    let unites = vue.blancs(UNITES_X, y - 1, CHIFFRE_L, CHIFFRE_H);
    let (u, score, _) = meilleur(&m.chiffres, &unites);
    if score < SCORE_CHIFFRE {
        return None;
    }
    let dizaines = vue.blancs(DIZAINES_X, y - 1, CHIFFRE_L, CHIFFRE_H);
    let mut frames = u as u8;
    if dizaines.iter().sum::<f32>() > 30.0 {
        let (d, score, _) = meilleur(&m.chiffres, &dizaines);
        if score < SCORE_CHIFFRE {
            return None;
        }
        frames += 10 * d as u8;
    }
    if frames == 0 {
        return None;
    }

    // La flèche (ou le « N » du neutre).
    let fleche = vue.blancs(DIRECTION_X, y - 7, DIRECTION_C, DIRECTION_C);
    let (d, score, ecart) = meilleur(&m.directions, &fleche);
    if score < SCORE_DIRECTION || ecart < ECART_DIRECTION {
        return None;
    }

    // Les boutons : des ronds de couleur (bleu = L, jaune = M, rouge = H),
    // avec un poing (P) ou un pied (K) dedans.
    let mut boutons = 0;
    for i in 0..4 {
        let cx = BOUTON_X + BOUTON_PAS * i;
        let cy = y + 9;
        let demi = BOUTON_C as i32 / 2;
        let mut couleur = [0f32; 3];
        let mut colores = 0;
        let mut lumiere = Vec::with_capacity(BOUTON_C * BOUTON_C);
        for dy in -demi..demi {
            for dx in -demi..demi {
                let p = vue.rgb(cx + dx, cy + dy);
                let (max, min) = (*p.iter().max().unwrap(), *p.iter().min().unwrap());
                if max - min > 80 {
                    colores += 1;
                }
                for c in 0..3 {
                    couleur[c] += p[c] as f32;
                }
                lumiere.push(p.iter().map(|&c| c as f32).sum::<f32>());
            }
        }
        let part = colores as f32 / (BOUTON_C * BOUTON_C) as f32;
        if part < 0.2 {
            break; // plus de bouton sur cette ligne
        }
        if part < 0.55 {
            return None; // à moitié coloré : autre chose passe devant
        }
        let n = (BOUTON_C * BOUTON_C) as f32;
        let [r, g, b] = couleur.map(|c| c / n);
        let force = if b > r + 40.0 && g > r + 40.0 {
            0 // L : bleu
        } else if r > b + 60.0 && g > b + 60.0 {
            1 // M : jaune
        } else if r > g + 60.0 && r > b + 60.0 {
            2 // H : rouge
        } else {
            return None;
        };
        let lumiere = centrer(&lumiere);
        let (p, k) = (m.poing.score(&lumiere), m.pied.score(&lumiere));
        let pied = if p >= SCORE_BOUTON && p > k + 0.2 {
            false
        } else if k >= SCORE_BOUTON && k > p + 0.2 {
            true
        } else {
            return None;
        };
        let bit = 1 << (force + if pied { 3 } else { 0 });
        if boutons & bit != 0 {
            return None;
        }
        boutons |= bit;
    }

    Some(LigneJeu {
        frames,
        direction: d as u8 + 1,
        boutons,
    })
}

/// Lit les lignes du haut, jusqu'à la première douteuse.
pub fn lire_lignes(vue: &Vue) -> Vec<LigneJeu> {
    let mut lignes = Vec::new();
    for k in 0..LIGNES_LUES {
        match lire_ligne(vue, k) {
            Some(l) => lignes.push(l),
            None => break,
        }
    }
    lignes
}

/// Une nouvelle ligne apparue dans l'historique du jeu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EntreeJeu {
    /// Heure (unités de 100 ns, horloge de la capture) de la première image qui la montre.
    pub vu: i64,
    /// Frame du jeu où elle commence. Ne se compare qu'avec une entrée du même `segment`.
    pub frame: u64,
    /// Change quand on perd le fil (historique caché, ligne à 99 frames...).
    pub segment: u32,
    pub direction: u8,
    pub boutons: u8,
    /// Boutons qui viennent d'être appuyés (absents de la ligne d'avant).
    pub nouveaux: u8,
}

/// Suit l'historique d'une image à l'autre pour repérer les nouvelles lignes.
#[derive(Debug, Default)]
pub struct Suivi {
    /// Les lignes lues à l'image d'avant.
    avant: Vec<LigneJeu>,
    /// Frame du jeu où commence la ligne du haut.
    debut: u64,
    segment: u32,
}

impl Suivi {
    /// Nouvelle image : `lignes` = les lignes lues (la plus récente d'abord), `temps` = son heure.
    /// Renvoie les lignes apparues depuis l'image d'avant, de la plus ancienne à la plus récente.
    pub fn image(&mut self, lignes: &[LigneJeu], temps: i64) -> Vec<EntreeJeu> {
        if lignes.len() < 2 {
            // Rien de fiable sur cette image (historique caché, menu...) : on attend.
            return Vec::new();
        }
        let Some(decalage) = self.decalage(lignes) else {
            if lignes.len() < 4 && !self.avant.is_empty() {
                // Peu de lignes lisibles (quelque chose passe devant) : on attend la suite
                // plutôt que de perdre le fil.
                return Vec::new();
            }
            // On ne retrouve pas les lignes d'avant : on repart de cette image.
            self.segment += 1;
            self.avant = lignes.to_vec();
            return Vec::new();
        };
        if decalage == 0 {
            self.avant = lignes.to_vec();
            return Vec::new();
        }

        // L'ancienne ligne du haut est maintenant à la place `decalage`, avec sa durée finale.
        self.avancer(lignes[decalage].frames);
        let mut nouvelles = Vec::new();
        for j in (0..decalage).rev() {
            let ligne = lignes[j];
            nouvelles.push(EntreeJeu {
                vu: 0,
                frame: self.debut,
                segment: self.segment,
                direction: ligne.direction,
                boutons: ligne.boutons,
                nouveaux: ligne.boutons & !lignes[j + 1].boutons,
            });
            if j > 0 {
                self.avancer(ligne.frames);
            }
        }
        // L'image montre la frame (début du haut + durée du haut - 1) : on en déduit
        // l'heure à laquelle chaque nouvelle ligne est apparue, même si on a raté des images.
        let affichee = self.debut + lignes[0].frames as u64 - 1;
        for e in &mut nouvelles {
            e.vu = temps - (affichee - e.frame) as i64 * FRAME;
        }
        self.avant = lignes.to_vec();
        nouvelles
    }

    fn avancer(&mut self, frames: u8) {
        self.debut += frames as u64;
        // Une ligne à 99 a peut-être duré plus : la suite n'est plus comparable.
        if frames >= 99 {
            self.segment += 1;
        }
    }

    /// De combien de lignes l'historique a descendu depuis l'image d'avant.
    fn decalage(&self, lignes: &[LigneJeu]) -> Option<usize> {
        let avant = &self.avant;
        if avant.is_empty() {
            return None;
        }
        (0..lignes.len().min(5)).find(|&m| {
            let haut = lignes[m];
            if !haut.meme_etat(&avant[0]) || haut.frames < avant[0].frames {
                return false;
            }
            // Les lignes plus anciennes ne bougent plus : elles doivent être identiques.
            let communes = (lignes.len() - m).min(avant.len());
            if communes < 2 && m > 0 {
                return false;
            }
            (1..communes).all(|j| lignes[m + j] == avant[j])
        })
    }
}

// --- Le panneau « Combo Damage » du mode entraînement (en haut, au milieu) ---
//
// Chaque coup qui touche fait monter les dégâts du combo ; ils reviennent à 0 quand le
// combo est fini. On lit le nombre, chiffre par chiffre : un tremblement de l'écran déplace
// le texte d'un pixel ou deux, mais le nombre lu reste le même.
// Le texte est blanc sur un bandeau sombre, « 1488 (2526) », aligné à droite.

/// Haut de la bande lue (un peu au-dessus du texte, pour les tremblements).
const DEGATS_Y: i32 = 202;
const DEGATS_H: i32 = 28;
const DEGATS_X0: i32 = 645;
const DEGATS_X1: i32 = 800;
/// En dessous, un signe ne ressemble à aucun chiffre : la lecture est refusée.
const SCORE_DEGAT: f32 = 0.75;

/// Les dégâts du combo (joueur 1), lus sur une image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LectureDegats {
    pub valeur: u32,
}

fn modeles_degats() -> &'static Vec<Option<Modele>> {
    static MODELES: OnceLock<Vec<Option<Modele>>> = OnceLock::new();
    MODELES.get_or_init(|| {
        CHIFFRES_DEGATS
            .iter()
            .map(|c| Some(Modele::new(c)))
            .collect()
    })
}

/// Lit les dégâts du combo. `None` si le panneau n'est pas affiché ou mal lu.
pub fn lire_degats(vue: &Vue) -> Option<LectureDegats> {
    let largeur = (DEGATS_X1 - DEGATS_X0) as usize;
    let hauteur = DEGATS_H as usize;
    let mut gris = vec![0u8; largeur * hauteur];
    let mut colonnes = vec![false; largeur];
    for dy in 0..hauteur {
        for dx in 0..largeur {
            let p = vue.rgb(DEGATS_X0 + dx as i32, DEGATS_Y + dy as i32);
            let g = *p.iter().min().unwrap();
            gris[dy * largeur + dx] = g;
            // Le texte est à ~240 ; le décor vu à travers le bandeau ne dépasse pas ~205.
            if g > 225 {
                colonnes[dx] = true;
            }
        }
    }
    // Les signes : des groupes de colonnes avec du blanc.
    let mut signes: Vec<(usize, usize)> = Vec::new();
    let mut debut = None;
    for (x, &c) in colonnes.iter().chain([&false]).enumerate() {
        match (c, debut) {
            (true, None) => debut = Some(x),
            (false, Some(d)) => {
                signes.push((d, x));
                debut = None;
            }
            _ => {}
        }
    }
    // Le panneau : « N (M) », 5 à 14 signes, le dernier « ) » vers x = 775-780.
    let fin_texte = DEGATS_X0 as usize + signes.last()?.1;
    if !(5..=14).contains(&signes.len()) || !(768..=792).contains(&fin_texte) {
        return None;
    }
    // Chaque signe doit avoir la forme d'un caractère : 3 à 11 px de large, 14 à 19 px de
    // haut. Les parenthèses sont plus hautes (19 px) que les chiffres (16 px).
    let mut formes = Vec::new();
    for &(x0, x1) in &signes {
        let lignes: Vec<usize> = (0..hauteur)
            .filter(|&dy| (x0..x1).any(|x| gris[dy * largeur + x] > 225))
            .collect();
        let (haut, bas) = (*lignes.first()?, *lignes.last()?);
        let h = bas - haut;
        if !(3..=11).contains(&(x1 - x0)) || !(13..=19).contains(&h) {
            return None;
        }
        formes.push((haut, x1 - x0 <= 5 && h >= 18));
    }
    // Le premier nombre : les signes avant « ( » ; le texte finit par « ) ».
    let fin = formes.iter().position(|&(_, p)| p)?;
    if fin == 0 || !formes[formes.len() - 1].1 {
        return None;
    }
    // Chaque chiffre : comparé aux modèles, centré, à partir de son haut.
    let modeles = modeles_degats();
    let mut valeur: u32 = 0;
    for (&(x0, x1), &(haut, _)) in signes.iter().zip(&formes).take(fin) {
        let cx = (x0 + x1) as i32 / 2;
        let mut echantillon = Vec::with_capacity(DEGAT_L * DEGAT_H);
        for dy in 0..DEGAT_H {
            for dx in 0..DEGAT_L as i32 {
                let x = cx - DEGAT_L as i32 / 2 + dx;
                let y = haut + dy;
                let g = if x >= 0 && (x as usize) < largeur && y < hauteur {
                    gris[y * largeur + x as usize]
                } else {
                    0
                };
                echantillon.push(g as f32 / 255.0);
            }
        }
        let echantillon = centrer(&echantillon);
        let (chiffre, score) = modeles
            .iter()
            .enumerate()
            .filter_map(|(i, m)| m.as_ref().map(|m| (i, m.score(&echantillon))))
            .max_by(|a, b| a.1.total_cmp(&b.1))?;
        if score < SCORE_DEGAT {
            return None;
        }
        valeur = valeur.checked_mul(10)?.checked_add(chiffre as u32)?;
    }
    Some(LectureDegats { valeur })
}

/// Ce que le panneau des dégâts nous apprend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Degats {
    /// Un coup vient de toucher (heure de l'image, dégâts du combo après ce coup).
    Coup(i64, u32),
    /// Le combo est fini : les dégâts sont revenus à 0.
    Fin(i64),
}

/// Suit les dégâts du combo d'une image à l'autre.
#[derive(Debug, Default)]
pub struct SuiviDegats {
    /// Dernière valeur lue (`None` : pas encore lu).
    valeur: Option<u32>,
}

impl SuiviDegats {
    pub fn image(&mut self, lecture: Option<LectureDegats>, temps: i64) -> Vec<Degats> {
        let Some(nouvelle) = lecture.map(|l| l.valeur) else {
            return Vec::new();
        };
        let Some(avant) = self.valeur.replace(nouvelle) else {
            // Premier passage (on vient d'arriver) : pas un coup.
            return Vec::new();
        };
        if nouvelle == avant {
            Vec::new()
        } else if nouvelle == 0 {
            vec![Degats::Fin(temps)]
        } else if nouvelle < avant {
            // Les dégâts d'un combo ne font que monter : s'ils baissent, le jeu n'est pas
            // repassé par 0 et un nouveau combo a commencé.
            vec![Degats::Fin(temps), Degats::Coup(temps, nouvelle)]
        } else {
            vec![Degats::Coup(temps, nouvelle)]
        }
    }
}

static DEGATS: Mutex<VecDeque<(u64, Degats)>> = Mutex::new(VecDeque::new());
static DEGATS_LUS: AtomicI64 = AtomicI64::new(0);

fn publier_degats(suivi: &mut SuiviDegats, vue: &Vue, temps: i64) {
    let lecture = lire_degats(vue);
    if lecture.is_some() {
        DEGATS_LUS.store(temps, Ordering::Relaxed);
    }
    let evenements = suivi.image(lecture, temps);
    if evenements.is_empty() {
        return;
    }
    if let Ok(mut journal) = DEGATS.lock() {
        for evenement in evenements {
            let numero = journal.back().map_or(0, |(n, _)| *n) + 1;
            journal.push_back((numero, evenement));
        }
        while journal.len() > JOURNAL_MAX {
            journal.pop_front();
        }
    }
}

/// Vrai si le panneau des dégâts a été lu dans la dernière seconde.
pub fn degats_visibles(maintenant: i64) -> bool {
    let derniere = DEGATS_LUS.load(Ordering::Relaxed);
    derniere > 0 && maintenant - derniere < 10_000_000
}

/// Les événements des dégâts arrivés depuis le dernier appel.
pub fn nouveaux_degats(dernier: &mut u64) -> Vec<Degats> {
    let Ok(journal) = DEGATS.lock() else {
        return Vec::new();
    };
    let nouveaux = journal
        .iter()
        .filter(|(n, _)| *n > *dernier)
        .map(|(_, e)| *e)
        .collect();
    if let Some((n, _)) = journal.back() {
        *dernier = *n;
    }
    nouveaux
}

/// Le journal des lignes du jeu, rempli par le thread de capture : (numéro, entrée).
static JOURNAL: Mutex<VecDeque<(u64, EntreeJeu)>> = Mutex::new(VecDeque::new());
/// Heure de la dernière image où l'historique a été lu (0 = jamais).
static DERNIERE_LECTURE: AtomicI64 = AtomicI64::new(0);
/// La ligne du haut de l'historique du jeu à la dernière image (la ligne en cours).
static LIGNE_DU_HAUT: Mutex<Option<LigneJeu>> = Mutex::new(None);

/// La ligne en cours dans l'historique du jeu, avec son compteur de frames tel qu'affiché.
pub fn ligne_du_haut() -> Option<LigneJeu> {
    LIGNE_DU_HAUT.lock().ok().and_then(|l| *l)
}
const JOURNAL_MAX: usize = 400;

/// Ce que le thread de capture suit d'une image à l'autre.
#[derive(Debug, Default)]
pub struct Lecteur {
    pub historique: Suivi,
    pub degats: SuiviDegats,
}

/// Lit une image du jeu : l'historique d'inputs et le panneau des dégâts.
pub fn lire_image(lecteur: &mut Lecteur, vue: &Vue, temps: i64) {
    publier_degats(&mut lecteur.degats, vue, temps);
    let suivi = &mut lecteur.historique;
    let lignes = lire_lignes(vue);
    if lignes.len() >= 2 {
        DERNIERE_LECTURE.store(temps, Ordering::Relaxed);
        if let Ok(mut haut) = LIGNE_DU_HAUT.lock() {
            *haut = lignes.first().copied();
        }
    }
    let nouvelles = suivi.image(&lignes, temps);
    if nouvelles.is_empty() {
        return;
    }
    if let Ok(mut journal) = JOURNAL.lock() {
        let mut numero = journal.back().map_or(0, |(n, _)| *n);
        for e in nouvelles {
            numero += 1;
            journal.push_back((numero, e));
        }
        while journal.len() > JOURNAL_MAX {
            journal.pop_front();
        }
    }
}

/// Vrai si l'historique du jeu a été lu dans la dernière seconde.
pub fn historique_visible(maintenant: i64) -> bool {
    let derniere = DERNIERE_LECTURE.load(Ordering::Relaxed);
    derniere > 0 && maintenant - derniere < 10_000_000
}

/// L'historique d'inputs ou le panneau des dégâts a été lu dans les `secondes` dernières
/// secondes : SF6 est en mode entraînement (ou dans un replay), pas en ligne ni dans un menu.
pub fn ecran_entrainement(maintenant: i64, secondes: i64) -> bool {
    let derniere = DERNIERE_LECTURE
        .load(Ordering::Relaxed)
        .max(DEGATS_LUS.load(Ordering::Relaxed));
    derniere > 0 && maintenant - derniere < secondes * 10_000_000
}

/// Les lignes du journal apparues depuis `temps` (pour le `.json` d'une vidéo).
pub fn entrees_depuis(temps: i64) -> Vec<EntreeJeu> {
    let Ok(journal) = JOURNAL.lock() else {
        return Vec::new();
    };
    journal
        .iter()
        .map(|(_, e)| *e)
        .filter(|e| e.vu >= temps)
        .collect()
}

/// Les entrées arrivées dans le journal depuis le dernier appel (`dernier` = dernier numéro lu).
pub fn nouvelles_entrees(dernier: &mut u64) -> Vec<EntreeJeu> {
    let Ok(journal) = JOURNAL.lock() else {
        return Vec::new();
    };
    let nouvelles: Vec<EntreeJeu> = journal
        .iter()
        .filter(|(n, _)| *n > *dernier)
        .map(|(_, e)| *e)
        .collect();
    if let Some((n, _)) = journal.back() {
        *dernier = *n;
    }
    nouvelles
}

/// Les boutons d'une étape de combo : « 236MK » -> MK, « DI » -> HP + HK.
/// 0 si l'étape n'a pas de bouton (Drive Rush par 66).
pub fn boutons_etape(notation: &str) -> u8 {
    match notation.trim_start_matches(|c: char| c.is_ascii_digit()) {
        "DI" => HP | HK,
        "DP" => MP | MK,
        "THROW" | "Throw" => LP | LK,
        bouton => bit(bouton),
    }
}

/// Un appui de notre côté : son heure et les boutons qui doivent être tenus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Appui {
    pub temps: i64,
    pub boutons: u8,
}

/// Retrouve chaque appui dans le journal du jeu et renvoie les écarts exacts (en frames
/// du jeu) entre appuis successifs. `latence` = délai entre un appui et son affichage.
/// Sinon, la raison : `Err((index de l'appui, explication))`.
pub fn ecarts_du_jeu(
    appuis: &[Appui],
    journal: &[EntreeJeu],
    latence: i64,
) -> Result<Vec<u64>, (usize, String)> {
    let mut trouvees: Vec<&EntreeJeu> = Vec::new();
    for (i, appui) in appuis.iter().enumerate() {
        let attendu = appui.temps + latence;
        let entree = journal
            .iter()
            // La ligne où tous les boutons sont tenus et où l'un d'eux vient d'être appuyé.
            .filter(|e| {
                e.boutons & appui.boutons == appui.boutons && e.nouveaux & appui.boutons != 0
            })
            .filter(|e| trouvees.last().is_none_or(|t| e.frame > t.frame))
            .min_by_key(|e| (e.vu - attendu).abs())
            .ok_or((i, "pas dans l'historique lu".to_string()))?;
        let ecart_ms = (entree.vu - attendu) / 10_000;
        if (entree.vu - attendu).abs() > 2 * FRAME {
            return Err((i, format!("pas au bon moment ({ecart_ms:+} ms)")));
        }
        if let Some(avant) = trouvees.last()
            && avant.segment != entree.segment
        {
            return Err((i, "historique perdu de vue juste avant".to_string()));
        }
        trouvees.push(entree);
    }
    Ok(trouvees
        .windows(2)
        .map(|p| p[1].frame - p[0].frame)
        .collect())
}

/// Les boutons en texte : « HP+HK », « - » si aucun.
pub fn noms_boutons(boutons: u8) -> String {
    let noms: Vec<&str> = [
        (LP, "LP"),
        (MP, "MP"),
        (HP, "HP"),
        (LK, "LK"),
        (MK, "MK"),
        (HK, "HK"),
    ]
    .iter()
    .filter(|(b, _)| boutons & b != 0)
    .map(|(_, n)| *n)
    .collect();
    if noms.is_empty() {
        "-".to_string()
    } else {
        noms.join("+")
    }
}

/// La durée exacte (en frames) de la ligne du jeu qui a cette direction et ces boutons,
/// et qui est apparue vers `attendu` (heure de la capture). `None` si elle n'est pas
/// retrouvée ou pas encore finie.
#[cfg(test)]
pub fn duree_dans_le_jeu(
    journal: &[EntreeJeu],
    attendu: i64,
    direction: u8,
    boutons: u8,
) -> Option<u64> {
    let i = ligne_du_jeu(journal, attendu, direction, boutons)?;
    duree_ligne(journal, i)
}

/// L'index, dans le journal, de la ligne du jeu qui a cette direction et ces boutons
/// et qui est apparue vers `attendu` (heure de la capture).
pub fn ligne_du_jeu(
    journal: &[EntreeJeu],
    attendu: i64,
    direction: u8,
    boutons: u8,
) -> Option<usize> {
    let (i, e) = journal
        .iter()
        .enumerate()
        .filter(|(_, e)| e.direction == direction && e.boutons == boutons)
        .min_by_key(|(_, e)| (e.vu - attendu).abs())?;
    ((e.vu - attendu).abs() <= 2 * FRAME).then_some(i)
}

/// La durée (en frames) de la ligne `i` du journal, si elle est finie.
pub fn duree_ligne(journal: &[EntreeJeu], i: usize) -> Option<u64> {
    let e = journal.get(i)?;
    let suivante = journal.get(i + 1)?;
    (suivante.segment == e.segment).then(|| suivante.frame - e.frame)
}

/// Vrai si le jeu passe directement de la ligne `avant` à la ligne `apres` du journal
/// (une ligne à nous entre les deux n'a pas été vue par le jeu).
pub fn lignes_consecutives(journal: &[EntreeJeu], avant: usize, apres: usize) -> bool {
    apres == avant + 1
        && journal
            .get(avant)
            .zip(journal.get(apres))
            .is_some_and(|(a, b)| a.segment == b.segment)
}

/// Estime le délai entre nos appuis et leur apparition à l'écran.
/// Chaque appui isolé (pas d'autre appui du même bouton autour) retrouvé dans le journal
/// donne une mesure ; on garde la médiane des dernières.
#[derive(Debug, Default)]
pub struct Latence {
    mesures: VecDeque<i64>,
}

impl Latence {
    const MAX: usize = 31;
    /// Au-delà, l'appui et la ligne ne vont pas ensemble.
    const LIMITE: i64 = 3_000_000;

    /// Une nouvelle ligne du jeu, comparée à nos appuis récents (heure, bouton).
    pub fn mesurer(&mut self, entree: &EntreeJeu, appuis: &[(i64, u8)]) {
        if entree.nouveaux.count_ones() != 1 {
            return;
        }
        let memes: Vec<i64> = appuis
            .iter()
            .filter(|(_, b)| *b == entree.nouveaux)
            .map(|(t, _)| *t)
            .collect();
        let candidats: Vec<i64> = memes
            .iter()
            .copied()
            .filter(|&t| t <= entree.vu && entree.vu - t < Self::LIMITE)
            .collect();
        let [t] = candidats[..] else {
            return;
        };
        // Isolé : aucun autre appui de ce bouton à moins de la limite, avant ou après.
        if memes
            .iter()
            .any(|&u| u != t && (u - t).abs() < Self::LIMITE)
        {
            return;
        }
        self.mesures.push_back(entree.vu - t);
        if self.mesures.len() > Self::MAX {
            self.mesures.pop_front();
        }
    }

    /// Le délai entre la lecture de la manette par le jeu et l'affichage de cette frame.
    /// Un appui attend la prochaine lecture (entre 0 et 1 frame, une demi-frame en moyenne),
    /// puis ce délai : c'est donc la latence médiane moins une demi-frame. La médiane résiste
    /// mieux que le plus petit écart aux images affichées un peu en avance ou en retard.
    pub fn delai_lecture(&self) -> Option<i64> {
        if self.mesures.len() < 8 {
            return None;
        }
        Some(self.valeur()? - FRAME / 2)
    }

    /// La latence estimée, s'il y a assez de mesures.
    pub fn valeur(&self) -> Option<i64> {
        if self.mesures.len() < 3 {
            return None;
        }
        let mut triees: Vec<i64> = self.mesures.iter().copied().collect();
        triees.sort();
        Some(triees[triees.len() / 2])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Une photo du jeu (morceau de 160x680 pris en (40, 200) d'une image 1080p), en BGRA.
    fn photo(numero: usize) -> (Vec<u8>, usize, usize) {
        let chemin = format!("tests/photos/historique_{numero}.png");
        let decodeur = png::Decoder::new(std::io::BufReader::new(
            std::fs::File::open(&chemin).unwrap(),
        ));
        let mut lecteur = decodeur.read_info().unwrap();
        let mut rgb = vec![0; lecteur.output_buffer_size().unwrap()];
        let info = lecteur.next_frame(&mut rgb).unwrap();
        assert_eq!(info.color_type, png::ColorType::Rgb);
        let mut bgra = Vec::new();
        for p in rgb[..info.buffer_size()].chunks(3) {
            bgra.extend_from_slice(&[p[2], p[1], p[0], 255]);
        }
        (bgra, info.width as usize, info.height as usize)
    }

    fn texte(l: &LigneJeu) -> String {
        let mut t = format!("{} {}", l.frames, l.direction);
        for (bit, nom) in [
            (LP, "LP"),
            (MP, "MP"),
            (HP, "HP"),
            (LK, "LK"),
            (MK, "MK"),
            (HK, "HK"),
        ] {
            if l.boutons & bit != 0 {
                t.push(' ');
                t.push_str(nom);
            }
        }
        t
    }

    fn lire_photo(numero: usize) -> Vec<Option<String>> {
        let (pixels, l, h) = photo(numero);
        let vue = Vue::morceau(&pixels, l, h, 40, 200);
        (0..19)
            .map(|k| lire_ligne(&vue, k).map(|l| texte(&l)))
            .collect()
    }

    #[test]
    fn la_zone_lue_donne_la_meme_lecture_que_l_image_entiere() {
        // La photo 4 replacée dans une image 1080p, puis lue sur la seule zone copiée.
        let (morceau, l, h) = photo(4);
        let (lj, hj) = (1920usize, 1080usize);
        let mut entiere = vec![0u8; lj * hj * 4];
        for y in 0..h {
            let cible = ((200 + y) * lj + 40) * 4;
            entiere[cible..cible + l * 4].copy_from_slice(&morceau[y * l * 4..(y + 1) * l * 4]);
        }
        let (x0, y0, x1, y1) = Vue::zone_lue(lj as u32, hj as u32);
        let (lz, hz) = ((x1 - x0) as usize, (y1 - y0) as usize);
        let mut zone = Vec::with_capacity(lz * hz * 4);
        for y in y0 as usize..y1 as usize {
            let debut = (y * lj + x0 as usize) * 4;
            zone.extend_from_slice(&entiere[debut..debut + lz * 4]);
        }
        let vue_entiere = Vue::jeu(&entiere, lj, hj, lj * 4);
        let vue_zone = Vue::zone(&zone, (lj as u32, hj as u32), (lz, hz), lz * 4);
        let lire = |v: &Vue| -> Vec<_> {
            (0..19)
                .map(|k| lire_ligne(v, k).map(|l| texte(&l)))
                .collect()
        };
        assert_eq!(lire(&vue_zone), lire(&vue_entiere));
        assert_eq!(lire(&vue_zone), lire_photo(4));
    }

    #[test]
    fn lit_la_photo_4_avec_drive_parry_et_drive_impact() {
        // MP+MK (Drive Parry / Drive Rush) et HP+HK (Drive Impact) sur une même ligne.
        let attendu = [
            "99 5",
            "4 5 MP MK",
            "79 5",
            "5 5 MP MK",
            "81 5",
            "5 5 MP MK",
            "99 5",
            "5 5 HP HK",
            "3 5",
            "11 6",
            "99 5",
            "3 5 MP",
            "1 5",
            "1 4",
            "2 1",
            "1 2",
            "1 5",
            "4 6",
            "2 6 MK",
        ];
        let lu = lire_photo(4);
        for (k, a) in attendu.iter().enumerate() {
            assert_eq!(lu[k].as_deref(), Some(*a), "ligne {k}");
        }
    }

    #[test]
    fn lit_la_photo_1() {
        let attendu = [
            "85 5", "5 5 LK", "99 5", "4 5 MK", "24 5", "6 5 LK", "20 5", "4 5 MP", "21 5",
            "6 5 LP", "30 5", "6 5 LP", "39 5", "5 5 LP", "29 5", "4 5 LP", "21 5", "4 5 LK",
            "99 5",
        ];
        let lu = lire_photo(1);
        for (k, a) in attendu.iter().enumerate() {
            assert_eq!(lu[k].as_deref(), Some(*a), "ligne {k}");
        }
    }

    #[test]
    fn lit_la_photo_2() {
        let attendu = [
            "99 5",
            "4 4",
            "5 5",
            "2 6",
            "1 3",
            "4 3 MK HK",
            "4 3",
            "2 6",
            "3 5",
            "5 8",
            "70 5",
            "6 4",
            "5 8",
            "99 5",
            "5 5 LK",
            "99 5",
            "4 5 MK",
            "24 5",
            "6 5 LK",
        ];
        let lu = lire_photo(2);
        for (k, a) in attendu.iter().enumerate() {
            assert_eq!(lu[k].as_deref(), Some(*a), "ligne {k}");
        }
    }

    #[test]
    fn photo_3_lignes_cachees_ignorees() {
        // « 3 HITS » et « PUNISH » passent devant les lignes 5 à 8 : elles peuvent être
        // ignorées, mais jamais mal lues.
        let cachees = [(5, "3 2"), (6, "4 6"), (7, "99 5"), (8, "3 4")];
        let attendu = [
            Some("88 5"),
            Some("4 5 HP"),
            Some("1 5"),
            Some("1 4"),
            Some("2 1"),
            None,
            None,
            None,
            None,
            Some("15 5"),
            Some("1 2 HK"),
            Some("1 3 HK"),
            Some("1 3"),
            Some("7 6"),
            Some("1 3"),
            Some("1 2"),
            Some("3 1"),
            Some("2 2"),
            Some("6 6"),
        ];
        let lu = lire_photo(3);
        for (k, a) in attendu.iter().enumerate() {
            match a {
                Some(a) => assert_eq!(lu[k].as_deref(), Some(*a), "ligne {k}"),
                None => {
                    let bon = cachees.iter().find(|c| c.0 == k).unwrap().1;
                    assert!(
                        lu[k].is_none() || lu[k].as_deref() == Some(bon),
                        "ligne {k} : {:?}",
                        lu[k]
                    );
                }
            }
        }
    }

    fn l(frames: u8, direction: u8, boutons: u8) -> LigneJeu {
        LigneJeu {
            frames,
            direction,
            boutons,
        }
    }

    #[test]
    fn suivi_des_nouvelles_lignes() {
        let mut s = Suivi::default();
        // 1re image : on prend les repères.
        assert!(
            s.image(&[l(10, 5, 0), l(4, 2, 0), l(6, 5, 0)], 0)
                .is_empty()
        );
        // La ligne du haut grandit : rien de nouveau.
        assert!(
            s.image(&[l(11, 5, 0), l(4, 2, 0), l(6, 5, 0)], FRAME)
                .is_empty()
        );
        // MK appuyé : nouvelle ligne, la neutre finit à 12 frames.
        let n = s.image(&[l(1, 5, MK), l(12, 5, 0), l(4, 2, 0)], 2 * FRAME);
        assert_eq!(n.len(), 1);
        assert_eq!((n[0].nouveaux, n[0].vu), (MK, 2 * FRAME));
        let debut_mk = n[0].frame;
        // La capture a sauté des images : deux nouvelles lignes d'un coup.
        let n = s.image(
            &[l(2, 3, HK), l(3, 3, 0), l(5, 5, MK), l(12, 5, 0)],
            9 * FRAME,
        );
        assert_eq!(n.len(), 2);
        assert_eq!(n[0].frame, debut_mk + 5);
        assert_eq!(n[1].frame, debut_mk + 8);
        assert_eq!(n[1].nouveaux, HK);
        // Montrée pour la 2e frame à 9 × FRAME : apparue une frame plus tôt.
        assert_eq!(n[1].vu, 8 * FRAME);
    }

    #[test]
    fn suivi_perd_le_fil_proprement() {
        let mut s = Suivi::default();
        s.image(&[l(10, 5, 0), l(4, 2, 0), l(6, 5, 0)], 0);
        let segment = s.segment;
        // Historique remis à zéro (reset de l'entraînement) : rien d'inventé.
        let apres_reset = [l(3, 5, 0), l(1, 6, 0), l(2, 5, 0), l(1, 4, 0)];
        assert!(s.image(&apres_reset, FRAME).is_empty());
        let n = s.image(
            &[l(1, 5, LP), l(3, 5, 0), l(1, 6, 0), l(2, 5, 0)],
            2 * FRAME,
        );
        assert_eq!(n.len(), 1);
        assert_ne!(n[0].segment, segment);
    }

    fn entree(vu: i64, frame: u64, boutons: u8, nouveaux: u8) -> EntreeJeu {
        EntreeJeu {
            vu,
            frame,
            segment: 0,
            direction: 5,
            boutons,
            nouveaux,
        }
    }

    #[test]
    fn ecarts_exacts_avec_le_mash() {
        let latence = 700_000; // 70 ms
        let journal = [
            entree(1_000_000, 100, HP, HP),
            entree(1_500_000, 103, HP | HK, HK), // Drive Impact
            entree(4_000_000, 118, MK, MK),      // mash : premier MK, ignoré par le trial
            entree(4_500_000, 121, MK, MK),      // le MK du trial
        ];
        let appuis = [
            Appui {
                temps: 1_000_000 - latence,
                boutons: HP,
            },
            Appui {
                temps: 1_500_000 - latence + 50_000,
                boutons: HP | HK,
            },
            Appui {
                temps: 4_500_000 - latence - 30_000,
                boutons: MK,
            },
        ];
        assert_eq!(ecarts_du_jeu(&appuis, &journal, latence), Ok(vec![3, 18]));
        // Un appui introuvable : pas d'écarts inventés.
        let appuis_loin = [
            appuis[0],
            Appui {
                temps: 9_000_000,
                boutons: MK,
            },
        ];
        assert!(ecarts_du_jeu(&appuis_loin, &journal, latence).is_err());
    }

    #[test]
    fn duree_d_une_ligne_du_jeu() {
        let mut journal = vec![entree(1_000_000, 100, 0, 0), entree(1_700_000, 104, MK, MK)];
        journal[0].direction = 4;
        // La ligne « ← » est retrouvée, elle a duré 4 frames.
        assert_eq!(duree_dans_le_jeu(&journal, 1_100_000, 4, 0), Some(4));
        // La ligne MK n'est pas encore finie.
        assert_eq!(duree_dans_le_jeu(&journal, 1_700_000, 5, MK), None);
        // Trop loin dans le temps : ce n'est pas la même.
        assert_eq!(duree_dans_le_jeu(&journal, 9_000_000, 4, 0), None);
    }

    fn photo_entiere(nom: &str) -> Option<(Vec<u8>, usize, usize)> {
        let chemin = format!("tests/photos/{nom}.png");
        let fichier = std::fs::File::open(&chemin).ok()?;
        let mut lecteur = png::Decoder::new(std::io::BufReader::new(fichier))
            .read_info()
            .unwrap();
        let mut rgb = vec![0; lecteur.output_buffer_size().unwrap()];
        let info = lecteur.next_frame(&mut rgb).unwrap();
        let mut bgra = Vec::new();
        for p in rgb[..info.buffer_size()].chunks(3) {
            bgra.extend_from_slice(&[p[2], p[1], p[0], 255]);
        }
        Some((bgra, info.width as usize, info.height as usize))
    }

    #[test]
    fn lit_le_panneau_des_degats() {
        // Morceaux de 200x40 pris en (620, 196) : « 0 (2526) », « 2148 (2526) », « 2400 (2526) ».
        for (nom, valeur) in [
            ("degats_0", 0),
            ("degats_720", 720),
            ("degats_2148", 2148),
            ("degats_2400", 2400),
        ] {
            let (pixels, l, h) = photo_entiere(nom).unwrap();
            // Tel quel, puis décalé d'un ou deux pixels (tremblement de l'écran).
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (-1, -2), (2, 2)] {
                let vue = Vue::morceau(&pixels, l, h, 620 + dx, 196 + dy);
                let lu = lire_degats(&vue).map(|l| l.valeur);
                assert_eq!(lu, Some(valeur), "{nom} décalé de ({dx}, {dy})");
            }
        }
        // Ailleurs (pas de panneau) : rien.
        let (pixels, l, h) = photo(1);
        assert_eq!(lire_degats(&Vue::morceau(&pixels, l, h, 40, 200)), None);
    }

    #[test]
    fn suivi_des_degats() {
        let mut s = SuiviDegats::default();
        let l = |valeur| Some(LectureDegats { valeur });
        assert_eq!(s.image(l(0), 0), vec![]); // « 0 » au départ
        assert_eq!(s.image(l(480), 10), vec![Degats::Coup(10, 480)]);
        assert_eq!(s.image(l(480), 20), vec![]);
        assert_eq!(s.image(l(1440), 30), vec![Degats::Coup(30, 1440)]);
        assert_eq!(s.image(None, 50), vec![]); // panneau caché un instant
        assert_eq!(s.image(l(1488), 60), vec![Degats::Coup(60, 1488)]);
        // Nouveau combo sans repasser par 0 : fin de l'ancien, 1er coup du nouveau.
        assert_eq!(
            s.image(l(720), 70),
            vec![Degats::Fin(70), Degats::Coup(70, 720)]
        );
        assert_eq!(s.image(l(0), 80), vec![Degats::Fin(80)]);
    }

    #[test]
    fn boutons_des_etapes() {
        assert_eq!(boutons_etape("236MK"), MK);
        assert_eq!(boutons_etape("4HP"), HP);
        assert_eq!(boutons_etape("DI"), HP | HK);
        assert_eq!(boutons_etape("DR"), 0);
    }

    #[test]
    fn latence_mediane_des_appuis_isoles() {
        let mut lat = Latence::default();
        let appuis = [
            (0, LP),
            (10_000_000, LP),
            (20_000_000, MK),
            (20_200_000, MK),
        ];
        for (vu, b) in [(700_000, LP), (10_800_000, LP), (20_900_000, MK)] {
            lat.mesurer(&entree(vu, 0, b, b), &appuis);
        }
        // Le MK n'est pas isolé (deux appuis proches) : seulement deux mesures.
        assert_eq!(lat.valeur(), None);
        lat.mesurer(&entree(30_750_000, 0, LP, LP), &[(30_000_000, LP)]);
        assert_eq!(lat.valeur(), Some(750_000));
        assert_eq!(lat.delai_lecture(), None);
        for i in 0..10 {
            let t = 100_000_000 + i * 10_000_000;
            lat.mesurer(&entree(t + 600_000 + i * 10_000, 0, LP, LP), &[(t, LP)]);
        }
        assert_eq!(lat.delai_lecture(), Some(lat.valeur().unwrap() - FRAME / 2));
    }
}
