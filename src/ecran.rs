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
    BOUTON_C, CHIFFRE_H, CHIFFRE_L, CHIFFRES, DIRECTION_C, DIRECTIONS, PIED, POING,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Le journal des lignes du jeu, rempli par le thread de capture : (numéro, entrée).
static JOURNAL: Mutex<VecDeque<(u64, EntreeJeu)>> = Mutex::new(VecDeque::new());
/// Heure de la dernière image où l'historique a été lu (0 = jamais).
static DERNIERE_LECTURE: AtomicI64 = AtomicI64::new(0);
const JOURNAL_MAX: usize = 400;

/// Lit une image du jeu et ajoute ses nouvelles lignes au journal.
pub fn lire_image(suivi: &mut Suivi, vue: &Vue, temps: i64) {
    let lignes = lire_lignes(vue);
    if lignes.len() >= 2 {
        DERNIERE_LECTURE.store(temps, Ordering::Relaxed);
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
/// `None` si un appui n'est pas retrouvé ou si le fil a été perdu entre deux appuis.
pub fn ecarts_du_jeu(appuis: &[Appui], journal: &[EntreeJeu], latence: i64) -> Option<Vec<u64>> {
    let mut trouvees: Vec<&EntreeJeu> = Vec::new();
    for appui in appuis {
        let attendu = appui.temps + latence;
        let entree = journal
            .iter()
            // La ligne où tous les boutons sont tenus et où l'un d'eux vient d'être appuyé.
            .filter(|e| {
                e.boutons & appui.boutons == appui.boutons && e.nouveaux & appui.boutons != 0
            })
            .filter(|e| trouvees.last().is_none_or(|t| e.frame > t.frame))
            .min_by_key(|e| (e.vu - attendu).abs())?;
        if (entree.vu - attendu).abs() > 2 * FRAME {
            return None;
        }
        trouvees.push(entree);
    }
    trouvees
        .windows(2)
        .map(|p| (p[0].segment == p[1].segment).then(|| p[1].frame - p[0].frame))
        .collect()
}

/// La durée exacte (en frames) de la ligne du jeu qui a cette direction et ces boutons,
/// et qui est apparue vers `attendu` (heure de la capture). `None` si elle n'est pas
/// retrouvée ou pas encore finie.
pub fn duree_dans_le_jeu(
    journal: &[EntreeJeu],
    attendu: i64,
    direction: u8,
    boutons: u8,
) -> Option<u64> {
    let (i, e) = journal
        .iter()
        .enumerate()
        .filter(|(_, e)| e.direction == direction && e.boutons == boutons)
        .min_by_key(|(_, e)| (e.vu - attendu).abs())?;
    if (e.vu - attendu).abs() > 2 * FRAME {
        return None;
    }
    let suivante = journal.get(i + 1)?;
    (suivante.segment == e.segment).then(|| suivante.frame - e.frame)
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
        assert_eq!(ecarts_du_jeu(&appuis, &journal, latence), Some(vec![3, 18]));
        // Un appui introuvable : pas d'écarts inventés.
        let appuis_loin = [
            appuis[0],
            Appui {
                temps: 9_000_000,
                boutons: MK,
            },
        ];
        assert_eq!(ecarts_du_jeu(&appuis_loin, &journal, latence), None);
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
    }
}
