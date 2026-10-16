# MusicApp — Plan de fabrication

Référence : `docs/superpowers/specs/2026-10-16-musicapp-design.md`

## Étape 0 — Mise en place

- Installer Rust (via mise, sans toucher au système).
- Créer le squelette de l'app Tauri (dossier `src-tauri` + un dossier `src` pour l'interface).
- Mettre le projet sous suivi de versions et exclure ce qui ne se versionne pas.

Point de contrôle : le squelette vide s'ouvre dans une fenêtre.

## Étape 1 — Le cerveau (côté Rust)

- Mémoriser le dossier de musique choisi (fichier de config au bon endroit sur Arch).
- Scanner le dossier en profondeur, repérer les fichiers média, ignorer le reste sans rien dire.
- Lire les métadonnées : titre, artiste, album, durée, pochette.
- Mettre le résultat en cache (fichier JSON) pour démarrer vite ensuite.
- Exposer tout ça à l'interface : choisir un dossier, relancer un scan, demander la liste, demander une pochette.

Point de contrôle : je peux scanner un dossier de test et voir la liste sortir côté Rust.

## Étape 2 — L'interface (layout A, thème sombre)

- Structure : barre latérale (Écouter / Bibliothèque / Vidéos), zone de contenu, barre de lecture en bas.
- Bibliothèque : liste virtualisée, recherche instantanée, tri par titre / artiste / album.
- Pochettes : vignettes dans la liste, image du morceau en cours en bas, dégradé propre si absente.
- Vue Écouter : morceau en cours en grand + file d'attente.
- Vue Vidéos : liste des mp4 et lecture dedans.
- Premier lancement sans dossier : un écran simple qui propose « Choisir un dossier ».

Point de contrôle : la fenêtre est jolie et navigable, même sans musique dedans.

## Étape 3 — La lecture

- Audio et vidéo lus dans la fenêtre, avec la barre de progression.
- Fichier suivant / précédent, clic pour lancer, pause/reprise.
- File d'attente qui avance seule.
- Shuffle et répéter (liste / morceau / arrêt).
- Touches média du clavier (play/pause, suivant, précédent).
- Dossier disparu : message discret, pas de plantage.

Point de contrôle : j'écoute un morceau du début à la fin, puis le suivant enchaîne seul.

## Étape 4 — Léger et installé

- Mesurer la mémoire au repos : on doit rester loin derrière Spotify.
- Corriger ce qui pèse inutilement.
- Compiler la version finale et poser l'icône + l'entrée dans le menu des applications.
- Écrire un mode d'emploi très court (où mettre ses fichiers, comment ouvrir l'app).

Point de contrôle : l'app s'ouvre d'un clic depuis le menu, sans terminal.

## Points de vigilance

- Si les mp4 ne s'affichent pas (codec manquant), repli sur `mpv`, déjà installé.
- Installer des paquets système est autorisé ; demander avant, si besoin est.
- Objectif de légèreté : ne rien ajouter de superflu à l'interface.
