# MusicApp — Note de design

Date : 2026-10-16
Statut : validée avec le client en conversation, puis relue par un œil neuf.

## Intention et pourquoi

Le client utilise Spotify sous Linux et deux choses le gênent : la consommation RAM et les publicités. Il veut un player local pour sa propre collection de musique, qu'il n'a pas encore mais qu'il va installer bientôt : « un dossier avec plein de musique ».

MusicApp est un player de fichiers audio/vidéo qui lit ce qu'il y a sur son disque. Aucune connexion réseau, aucun compte, aucune pub.

## Ce que dit le client (décisions prises avec lui)

- Player pour fichiers **mp3** (audio) et **mp4** (vidéos musicales, avec image).
- Bibliothèque alimentée par un **dossier** qu'il pointe une fois ; scannage récursif, l'app trouve tout ce qu'il y a dedans. Un bouton « rescan » récupère les fichiers ajoutés ensuite.
- Interface retenue : **maquette A** (visible dans `.superpowers/brainstorm/87917-…/content/layout.html`) — barre latérale à gauche, liste des titres avec recherche, barre de lecture en bas.
- **Thème sombre**, à la Spotify mais épuré.
- Fichiers non supportés (pdf, txt, etc.) présents dans le dossier : **ignorés silencieusement**, sans message ni affichage.
- Moteur Tauri : approuvé.
- Recherche instantanée et tri de la liste : approuvés dans la maquette.
- File d'attente qui avance toute seule vers le morceau suivant : approuvée dans la maquette.
- **Mélange aléatoire (shuffle), bouton répéter et touches média du clavier : oui, dès la v1.**
- **Installation de paquets système autorisée pendant la mise en place** si les codecs manquent (repli possible : `mpv`, déjà présent).

## Objectifs

- Une fenêtre : côté gauche Écouter / Bibliothèque / Vidéos, à droite le contenu, en bas la barre de lecture.
- Lecture audio et vidéo **dans la fenêtre** (vue dédiée pour les vidéos).
- Léger : viser nettement moins de RAM que Spotify (~400–700 Mo) ; cap indicatif ~100–150 Mo en usage normal. C'est un cap, pas une course.
- Sélection du dossier de musique au premier lancement ; le choix est mémorisé (fichier de config dans le répertoire standard de l'OS).
- Index de bibliothèque mis en cache (JSON dans le répertoire standard de l'OS) pour démarrer vite.
- App lançable depuis le menu des applications, pas seulement en ligne de commande.

## Non-objectifs (v1)

- Pas de playlists personnalisables (post-v1).
- Pas de lecture d'un flux réseau, pas de streaming, pas d'égaliseur, pas d'éditeur de tags.
- Pas d'import M3U ni de réorganisation des fichiers sur le disque.

## Anti-objectifs (ce qui serait un échec même si ça marche)

- Utiliser plus de RAM que Spotify.
- Demander un compte ou toucher au réseau.
- Une interface « admin système » austère : le client veut du joli.

## Contraintes

- Machine cible : Arch / Omarchy (Hyprland, Wayland).
- Outils disponibles : Node 26 (mise), mpv installé ; **pas encore de Rust** (à installer pendant la mise en place du projet).
- Client non technique : les explications et le mode d'emploi restent sans jargon.

## Choix techniques (tranchés avec le client)

- **Tauri 2** (Rust) + frontend web minimal (HTML/CSS/JS vanilla, pas de framework).
- Lecture audio/vidéo via les balises HTML5 `audio`/`video` dans la webview (GStreamer/WebKitGTK côté système).
- Scannage du dossier et lecture des métadonnées (titre, artiste, durée, pochette) côté Rust, avec la bibliothèque `lofty`.
- Mélange aléatoire, répétition (liste / morceau / arrêt) et touches média via MPRIS.

## Décisions laissées au builder (faites pendant la relecture, à vérifier en lisant)

1. **Formats.** On lit tout ce que le moteur système sait décoder (mp3, m4a, flac, ogg, opus, wav, mp4, webm…) : ça ne coûte rien et ça évite la mauvaise surprise d'un dossier à moitié muet. Seuls les vrais non-médias (pdf, txt, jpg…) sont ignorés en silence.
2. **Taille de bibliothèque.** On vise la collection personnelle : quelques milliers de titres. La liste est donc virtualisée (on ne dessine que ce qui est visible) et le scan tourne sans bloquer la fenêtre.
3. **Pochettes.** Pochette extraite pour le morceau en cours (barre du bas) et vignettes dans la liste, extraites à la demande pour ne pas ralentir le scan. Pochette absente : dégradé sobre, pas de trou visuel.
4. **Vue « Écouter ».** Accueil : morceau en cours en grand, la file d'attente, et reprise rapide. « Bibliothèque » reste la liste complète.
5. **Dossier absent** (disque externe débranché). L'app s'ouvre normalement, la liste s'affiche, et une ligne discrète propose de rebrancher le disque ou de choisir un autre dossier. Pas de plantage, pas de fenêtre d'erreur.
6. **Livraison.** Une version « release » compilée + une entrée dans le menu des applications (fichier `.desktop` et icône), pour ouvrir l'app d'un clic.
7. **Sélection du dossier.** Un bouton « Choisir un dossier » ouvre le sélecteur système ; le chemin est mémorisé et re-scannable.
8. **Contrôles.** Un clic lance un morceau ; les boutons shuffle / répéter vivent dans la barre de lecture.
