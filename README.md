# Écho

Un lecteur de musique et de vidéos local pour Linux : léger, sans publicité, sans compte, sans réseau.
Tu lui montres un dossier, il joue tout ce qu'il contient.

## Ce que ça fait

- Import d'un ou plusieurs dossiers : l'app les scanne en profondeur et construit ta bibliothèque (titre, artiste, album, durée, pochette).
- Audio (mp3, flac, m4a, ogg, opus, wav…) et vidéos (mp4, mkv, webm…), tout ce que ton système sait lire.
- Les fichiers non-média du dossier sont ignorés en silence, et le bouton **Actualiser** prend en compte les nouveautés.
- Recherche instantanée, tri par titre / artiste / album / durée.
- Lecture dans la fenêtre, mélange aléatoire, répétition, reprise là où tu t'étais arrêté.
- Trois dispositions au choix : **Normale** (bibliothèque + file d'attente), **Compacte** (pochettes en carrousel), **Minimaliste** (album ouvert et sa liste).
- Thème clair ou sombre, quatre palettes de couleurs, trois polices, taille du texte et arrondis réglables.
- Tout se règle dans **Réglages** (lecture, bibliothèque, formats acceptés, démarrage automatique, sauvegarde des réglages).

## Installation

```bash
./packaging/build.sh     # compile (une fois, puis à chaque mise à jour)
./packaging/install.sh   # pose l'icône et l'entrée dans le menu des applications
```

Aucun droit administrateur : tout va dans ton dossier personnel (`~/.local`).
Ensuite, Écho s'ouvre depuis le menu des applications, ou en tapant `echo`.

## Utilisation

1. Au premier lancement, clique sur **Ajouter un dossier** et sélectionne le dossier qui contient ta musique.
2. Clique sur un morceau pour l'écouter. Les suivants s'enchaînent tout seuls.
3. Si tu ajoutes des fichiers plus tard, clique sur **Actualiser**.
4. Dans **Réglages**, choisis la disposition, les couleurs et la police qui te plaisent.

## Raccourcis

- `Espace` : lecture / pause
- `Maj + →` : morceau suivant
- `Maj + ←` : morceau précédent
- `←` / `→` : reculer / avancer de quelques secondes
- `Échap` : fermer la vue vidéo ou un panneau
- Les touches média du clavier (play/pause, suivant, précédent) pilotent aussi l'app.

## Pour les curieux (technique)

- Interface : HTML / CSS / JavaScript dans une fenêtre Tauri 2 (moteur d'affichage du système).
- Cœur : Rust. Scannage des dossiers, lecture des métadonnées (`lofty`), pochettes mises en cache, MPRIS pour les touches média et l'icône du système.
- Ta musique ne quitte jamais ta machine ; l'app ne fait aucune connexion réseau.
- Les réglages vivent dans `~/.config/com.echo.player/`, le cache des pochettes dans `~/.cache/com.echo.player/`.
- Le design et le plan de fabrication sont dans `docs/superpowers/`.
