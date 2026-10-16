<div align="center">

<img src="src-tauri/icons/128x128@2x.png" width="112" alt="Icône d'Écho" />

# Écho

**Un lecteur de musique local pour Linux : léger, sans publicité, sans compte, sans réseau.**

Tu lui montres un dossier, il joue tout ce qu'il contient.

![Tauri 2](https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white)
![Rust](https://img.shields.io/badge/Rust-1.77+-000000?logo=rust&logoColor=white)
![Linux](https://img.shields.io/badge/Linux-FCC624?logo=linux&logoColor=black)
![JavaScript](https://img.shields.io/badge/JavaScript-vanilla-F7DF1E?logo=javascript&logoColor=black)
![Licence MIT](https://img.shields.io/badge/licence-MIT-green)

</div>

---

## Sommaire

- [Fonctionnalités](#fonctionnalités)
- [Installation](#installation)
- [Utilisation](#utilisation)
- [Raccourcis clavier](#raccourcis-clavier)
- [Formats pris en charge](#formats-pris-en-charge)
- [Architecture](#architecture)
- [Développement](#développement)
- [Données et vie privée](#données-et-vie-privée)
- [Licence](#licence)

## Fonctionnalités

| | |
|---|---|
| 📁 **Bibliothèque** | Un ou plusieurs dossiers analysés en profondeur : titre, artiste, album, durée, pochette. Les analyses suivantes ne relisent que les fichiers modifiés. |
| 🔎 **Recherche et tri** | Recherche instantanée, sans tenir compte des accents. Tri par titre, artiste, album ou durée. Reste fluide avec des dizaines de milliers de morceaux. |
| ▶️ **Lecture** | Aléatoire (chaque morceau passe une fois), répétition de la file ou d'un titre, reprise au morceau et à la position de la dernière fois. |
| 🎨 **Trois dispositions** | **Normale** (bibliothèque et file d'attente), **Compacte** (pochettes en carrousel), **Minimaliste** (un album ouvert). S'adaptent aux fenêtres étroites des gestionnaires en mosaïque (Hyprland, Sway…). |
| ✏️ **Modifier un morceau** | Titre, artiste, album et pochette, par clic droit ou avec le bouton **Modifier**. Tes fichiers ne sont jamais modifiés, et tu peux toujours revenir à l'original. |
| 🌗 **Apparence** | Thème clair, sombre ou celui du système. Quatre palettes, trois polices, taille du texte et arrondis réglables, animations désactivables. |
| ⌨️ **Intégration au bureau** | Touches média et widget du bureau (MPRIS), démarrage avec la session, entrée dans le menu des applications. |

## Installation

Prérequis : [Rust](https://rustup.rs) et les dépendances système de Tauri 2 (WebKitGTK). Voir le [guide Tauri](https://v2.tauri.app/start/prerequisites/#linux).

```bash
git clone https://github.com/wave3k/echo-player.git
cd echo-player
./packaging/build.sh     # compile la version optimisée
./packaging/install.sh   # installe le programme, l'icône et l'entrée de menu
```

Aucun droit administrateur n'est nécessaire : tout est installé dans `~/.local`.
Écho s'ouvre ensuite depuis le menu des applications, ou avec la commande `echo-player`.

## Utilisation

1. Au premier lancement, clique sur **Ajouter un dossier** et choisis celui qui contient ta musique.
2. Clique sur un morceau : les suivants s'enchaînent tout seuls.
3. Tu as ajouté des fichiers ? Clique sur **Actualiser**. Tu peux aussi activer l'actualisation au démarrage.
4. Dans **Réglages**, choisis la disposition, les couleurs et la police.

## Raccourcis clavier

| Touche | Action |
|---|---|
| `Espace` | Lecture / pause |
| `Maj` + `→` / `←` | Morceau suivant / précédent |
| `→` / `←` | Avancer / reculer de quelques secondes (5, 10 ou 30 s) |
| `Échap` | Fermer une fenêtre ou un panneau |
| Touches média | Lecture, pause, suivant, précédent |
| `Tab` | Déplacer le focus. Les barres de progression se règlent avec `←` / `→`. |

## Formats pris en charge

mp3, flac, m4a, aac, ogg, opus, wav, aiff, wma, ainsi que d'autres formats moins courants (wv, ape, mpc…).
Les conteneurs vidéo (mp4, webm, mkv, mov) sont lus pour leur **piste son uniquement**.

Tu peux choisir les formats dans **Réglages → Formats acceptés**. Le décodage passe par GStreamer : un format peu courant peut demander un greffon (`gst-plugins-good`, `gst-libav`).

## Architecture

```
src/                 Interface : HTML, CSS et JavaScript, sans framework ni étape de build
  app.js             Affichage, listes virtualisées, réglages, lecteur
  logic.js           Règles de lecture (suivant, aléatoire, répétition), testées à part
src-tauri/src/       Cœur en Rust
  main.rs            Commandes Tauri, état partagé, démarrage
  library.rs         Analyse des dossiers, tags et pochettes (lofty)
  store.rs           Réglages, cache et corrections : fichiers JSON écrits de façon atomique
  mpris.rs           Touches média et widget du bureau (souvlaki)
packaging/           Compilation, installation et entrée .desktop
tests/               Tests des règles de lecture (node --test)
```

Quelques choix de conception :

- **Les fichiers audio ne sont jamais modifiés.** Les corrections de titre, d'artiste ou de pochette sont gardées dans `overrides.json` et appliquées à l'affichage.
- **Le démarrage est rapide.** La bibliothèque est d'abord lue depuis le cache, puis rafraîchie en arrière-plan. Un seul scan tourne à la fois.
- **Les pochettes sont lues sur le disque.** Elles passent par le protocole d'asset de Tauri, limité aux dossiers de musique et au cache, et ne sont jamais encodées en base64.
- **L'interface reste réactive.** Les listes sont virtualisées, le tri est mémorisé, et les travaux lourds tournent hors du thread principal.

## Développement

```bash
cd src-tauri
cargo run              # lance l'app en mode développement (lit directement src/)
cargo test             # tests du cœur : scan, pochettes, formats, réglages, cache
cargo clippy --all-targets

cd ..
node --test tests/     # tests des règles de lecture
```

## Données et vie privée

Écho ne fait **aucune connexion réseau** : ta musique et tes réglages restent sur ta machine.

| Emplacement | Contenu |
|---|---|
| `~/.config/com.echo.player/` | Réglages, cache de la bibliothèque, corrections des morceaux |
| `~/.cache/com.echo.player/covers/` | Pochettes extraites des fichiers (peut être vidé depuis les réglages) |

Si un fichier de réglages est abîmé, il est mis de côté (sous le nom `settings.json.bad`) plutôt qu'effacé.

## Licence

Distribué sous licence [MIT](LICENSE).
