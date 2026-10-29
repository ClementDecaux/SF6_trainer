# SF6 Combos Trainer

Overlay de trials pour Street Fighter 6, en Rust. Il s'affiche par-dessus le jeu, passe à l'étape
suivante quand tu fais le bon coup, et montre ton historique d'inputs à la frame près.

L'outil ne touche jamais au jeu : il lit seulement ta manette (aucun risque avec l'anti-cheat).

## Lancer

Windows, avec Rust installé ([rustup.rs](https://rustup.rs)) :

```powershell
cargo run          # lance le 1er combo de combos.toml
cargo run -- 2     # lance le 2e
cargo test         # lance les tests
```

- SF6 doit être en **plein écran fenêtré** (borderless), sinon rien ne peut s'afficher par-dessus.
- Le contrôleur doit être en mode **XInput** (mode manette Xbox).
- Ctrl+C dans le terminal pour fermer l'overlay.
- L'outil suppose que tu joues à gauche de l'écran (b = 4 = vers la gauche).

## Fichiers de config

| Fichier | Contenu |
| --- | --- |
| `combos.toml` | Tes combos, en notation forum : `b+hp xx DI xx b+mk xx qcf+mk, b+mk xx hcb+mp` |
| `controles.toml` | Quel bouton physique fait quoi dans SF6 (LP, MP, HP...) |
| `overlay.toml` | Position, taille et opacité de l'overlay |

Notation reconnue : directions `b f d u db df cr. st.` ou chiffres numpad, mouvements
`qcf qcb hcf hcb dp` ou `236 214 41236 63214 623 421`, `DI` (HP+HK), `DP` (MP+MK),
`DR` (66), `xx` ou `>` pour un cancel, `,` pour un link.

## Organisation du code

| Fichier | Rôle |
| --- | --- |
| `src/main.rs` | Lancement : charge les configs, ouvre l'overlay, démarre le thread de la manette |
| `src/entrees.rs` | Thread de la manette : lit les inputs, reconnaît les coups, fait avancer le trial |
| `src/notation.rs` | Directions -> notation numpad (1 à 9) |
| `src/motion.rs` | Reconnaissance des mouvements spéciaux (236, 63214, 66...) |
| `src/combo.rs` | Lecture des combos écrits en notation forum, moteur de trial |
| `src/controles.rs` | Correspondance boutons physiques -> actions SF6 |
| `src/partage.rs` | État partagé entre le thread de la manette et l'overlay |
| `src/overlay.rs` | Fenêtre de l'overlay (egui) |
| `src/affichage.rs` | Historique d'inputs coloré dans le terminal |

## Apprendre Rust avec ce projet

- [Le Rust Book](https://doc.rust-lang.org/book/) ([en français](https://jimskapt.github.io/rust-book-fr/)) et [Rustlings](https://github.com/rust-lang/rustlings).
- `struct`, `enum`, `match` : `notation.rs`, `motion.rs`
- Erreurs avec `Result` : `combo.rs` (`parser`, `normaliser`)
- Tests : les blocs `#[cfg(test)]` en bas de chaque fichier
- Threads et `Arc<Mutex<_>>` : `main.rs`, `partage.rs`
