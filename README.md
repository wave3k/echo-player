# MusicApp

Un lecteur de musique et de vidéos local pour Linux : léger, sans publicité, sans compte, sans réseau.
Tu lui montres un dossier, il joue tout ce qu'il contient.

## Ce que ça fait

- Import d'un dossier : l'app le scanne en profondeur et construit ta bibliothèque (titre, artiste, album, durée, pochette).
- Audio (mp3, flac, m4a, ogg, opus, wav…) et vidéos (mp4, mkv, webm…), tout ce que ton système sait lire.
- Recherche instantanée, tri par titre / artiste / album / durée.
- Lecture dans la fenêtre, avec mélange aléatoire, répétition et touches média du clavier.
- Les fichiers non-musique du dossier sont ignorés en silence.

## Installation

```bash
./packaging/build.sh     # compile (une fois, puis à chaque mise à jour)
./packaging/install.sh   # pose l'icône et l'entrée dans le menu des applications
```

Aucun droit administrateur : tout va dans ton dossier personnel (`~/.local`).
Ensuite, MusicApp s'ouvre depuis le menu des applications, ou en tapant `musicapp`.

## Utilisation

1. Au premier lancement, clique sur **Choisir un dossier** et sélectionne le dossier qui contient ta musique.
2. Clique sur un morceau pour l'écouter. Les suivants s'enchaînent tout seuls.
3. Si tu ajoutes des fichiers plus tard, clique sur **Actualiser**.
4. Pour changer de dossier, clique sur **Changer de dossier**.

## Raccourcis

- `Espace` : lecture / pause
- `Maj + →` : morceau suivant
- `Maj + ←` : morceau précédent
- Les touches média du clavier (play/pause, suivant, précédent) pilotent aussi l'app.

## Pour les curieux (technique)

- Interface : HTML / CSS / JavaScript dans une fenêtre Tauri 2 (moteur d'affichage du système).
- Cœur : Rust. Scannage du dossier, lecture des métadonnées (`lofty`), pochettes mises en cache, MPRIS pour les touches média.
- Ta musique ne quitte jamais ta machine ; l'app ne fait aucune connexion réseau.
- Le design et le plan de fabrication sont dans `docs/superpowers/`.
