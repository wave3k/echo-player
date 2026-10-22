<div align="center">

<img src="src-tauri/icons/128x128@2x.png" width="112" alt="Icône d'Écho" />

# Écho

**Un lecteur de musique local pour Linux et Windows : léger, sans publicité, sans compte, sans réseau.**

Tu lui montres un dossier, il joue tout ce qu'il contient.

![Tauri 2](https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white)
![Rust](https://img.shields.io/badge/Rust-1.77+-000000?logo=rust&logoColor=white)
![Linux](https://img.shields.io/badge/Linux-FCC624?logo=linux&logoColor=black)
![Windows](https://img.shields.io/badge/Windows-0078D6?logo=windows&logoColor=white)
![JavaScript](https://img.shields.io/badge/JavaScript-vanilla-F7DF1E?logo=javascript&logoColor=black)
![Licence MIT](https://img.shields.io/badge/licence-MIT-green)

</div>

---

## Sommaire

- [Fonctionnalités](#fonctionnalités)
- [À venir](#à-venir)
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
| 📁 **Bibliothèque** | Un ou plusieurs dossiers analysés en profondeur : titre, artiste, album, durée, pochette. Les nouveaux fichiers apparaissent tout seuls. Les analyses suivantes ne relisent que les fichiers modifiés. |
| 🎞️ **Clips** | Une section pour regarder tes clips (webm, mp4, mkv) : la vidéo est calée sur le son, qui garde le fondu enchaîné, l'égaliseur et les touches média. En plein écran avec **F**, et dans une mini-fenêtre qui reste à l'écran quand tu changes de section (on la ferme d'un clic). La vidéo n'est décodée que quand elle est visible. L'artiste est deviné dans le titre (« Artiste - Titre (Official Video) ft. Invité » → *Artiste* / *Titre (feat. Invité)*). Un téléchargement incomplet de yt-dlp (« .f616.webm », l'image sans le son) est signalé. |
| 🎤 **Featurings** | Les invités sont reconnus dans l'artiste (« Tiakola x Dave », « Ninho feat. Niska ») et dans le titre (« (feat. Dave) ») : chaque artiste a sa liste, featurings compris, et le clic droit sur un morceau mène à ses artistes. |
| 🔊 **Son** | Volume égal (niveau des plateformes de streaming, sans saturer), égaliseur à 6 préréglages (Neutre, Basses, Rap, Voix, Aigus, Acoustique), fondu enchaîné de 3, 6 ou 10 s. |
| 🎤 **Paroles** | Paroles synchronisées en plein écran (touche **L**) : la ligne chantée s'illumine, un clic sur une ligne y saute. Lues dans un fichier `.lrc` à côté du morceau, dans les tags du fichier, ou en ligne (à activer : seuls l'artiste, le titre et la durée sont envoyés) sur [LRCLIB](https://lrclib.net), base libre de paroles synchronisées, puis lyrics.ovh en secours (texte seul). Les noms de clips YouTube sont décomposés (« Tiakola x Dave - Meridian (Clip officiel) ») pour retrouver le bon morceau. Un clip plus long que le son (intro, scènes) se cale en un clic : on touche la ligne au moment où elle est chantée, et le calage est retenu. |
| 🖼️ **Pochettes en ligne** | Pour les morceaux sans pochette, ou dont la pochette est une image de clip (à activer) : recherche sur [MusicBrainz](https://musicbrainz.org) et le [Cover Art Archive](https://coverartarchive.org), bases libres et sans compte, puis Deezer en secours. Une pochette n'est posée que si l'artiste, le titre et la durée correspondent, et jamais à la place d'une pochette choisie à la main. Au choix, celle de l'album ou celle du single (« Auto » : l'album s'il est dans les tags). Clic droit → « Chercher une pochette en ligne… » pour choisir parmi plusieurs propositions. |
| 💿 **Albums et artistes** | Dans la Bibliothèque, les onglets **Albums** et **Artistes** : une grille de pochettes, puis la page d'un album (pistes dans l'ordre du disque, année, durée) ou d'un artiste (ses plus écoutés, ses albums, ses morceaux et ses featurings, ses clips). Le clic droit sur un morceau et l'artiste du morceau en cours y mènent. Photos des artistes depuis Deezer, si tu les actives. |
| 📃 **Playlists** | Crée tes playlists, ajoute un morceau par clic droit → « Ajouter à une playlist », lis-les dans l'ordre ou en aléatoire. Importe et exporte-les au format M3U (VLC, foobar2000, Winamp, baladeurs…) : une playlist venue d'un autre ordinateur est retrouvée par nom de fichier ou par « Artiste - Titre ». |
| ❤️ **Favoris et listes automatiques** | Un cœur pour aimer un morceau, et des listes qui se remplissent seules : favoris, ajoutés récemment, les plus écoutés, écoutés récemment, jamais écoutés, et une liste par artiste. Une écoute compte après 30 secondes ; ces statistiques restent sur ta machine. |
| 🌊 **Visualiseur** | Le son mis en image, en barres ou en onde, dans « Écouter » et derrière les paroles. Calculé seulement quand il est à l'écran. |
| 🧹 **Doublons** | **Réglages → Bibliothèque → Trouver les doublons** : le même morceau en plusieurs fichiers (MP3 et FLAC, clip téléchargé deux fois…), avec le fichier de meilleure qualité signalé. Un doublon se masque d'un clic ; le fichier n'est jamais supprimé, et on peut le réafficher. |
| 🔎 **Recherche et tri** | Recherche instantanée, sans tenir compte des accents. Tri par titre, artiste, album ou durée. Reste fluide avec des dizaines de milliers de morceaux. |
| 🧠 **Recherche avancée** | Des filtres à combiner : `artiste:ninho`, `année>2018`, `durée<3:00`, `écoutes>10`, `ajouté<30j`, `est:favori`, `-live`… (le bouton **?** les rappelle). Une recherche se garde en **playlist intelligente**, qui se remplit seule, même avec les morceaux ajoutés plus tard. |
| ▶️ **Lecture** | Aléatoire (chaque morceau passe une fois), répétition de la file ou d'un titre, reprise au morceau et à la position de la dernière fois. |
| 📋 **File d'attente** | Glisse un morceau pour le déplacer (ou **Alt** + **↑** / **↓**), retire-le d'un clic, vide la file en gardant le morceau en cours, ou enregistre-la comme playlist. |
| 🎛️ **Vitesse et effets** | Vitesse de 0,5× à 2×, en gardant la voix ou avec la tonalité qui suit, comme un disque (« slowed », « sped up »), réverbération, et le **volume propre à un morceau** trop fort ou trop faible, gardé pour lui. Préréglages *Slowed + reverb* et *Sped up* ; la vidéo des clips suit la vitesse. |
| 🎨 **Trois dispositions** | **Normale** (bibliothèque et file d'attente), **Compacte** (pochettes en carrousel), **Minimaliste** (un album ouvert). S'adaptent aux fenêtres étroites des gestionnaires en mosaïque (Hyprland, Sway…). |
| ✏️ **Modifier un morceau** | Titre, artiste, album et pochette, par clic droit ou avec le bouton **Modifier**. Tes fichiers ne sont jamais modifiés, et tu peux toujours revenir à l'original. |
| 🌗 **Apparence** | Thème clair, sombre ou celui du système. Quatre palettes, trois polices, taille du texte et arrondis réglables, animations désactivables. |
| ⌨️ **Intégration au bureau** | Touches média et widget du bureau (MPRIS), icône dans la barre du système, notification à chaque morceau, statut Discord « Écoute… » (activable), démarrage avec la session. |
| 🔄 **Mises à jour** | Vérifiées toutes les 6 heures et installées toutes seules (ou juste signalées, au choix). Chaque version est signée : Écho refuse un fichier qui ne vient pas de lui. Windows, AppImage et installation par `install.sh` se mettent à jour d'un clic ; le paquet `.deb` passe par la page des versions. |

## À venir

| | |
|---|---|
| 🎚️ **Studio de transitions** | Des transitions façon DJ entre les morceaux : tempo et temps analysés, plusieurs styles, réglage à la main sur la forme d'onde, et une transition par enchaînement dans les playlists. |

## Installation

### Télécharger

Les fichiers sont sur la page [Releases](https://github.com/wave3k/echo-player/releases/latest) :

| Système | Fichier |
|---|---|
| Windows 10 / 11 | `Echo_x.y.z_x64-setup.exe` (installateur) ou `.msi` |
| Debian, Ubuntu, Mint… | `Echo_x.y.z_amd64.deb` : `sudo apt install ./Echo_*.deb` |
| Autres distributions Linux | `Echo_x.y.z_amd64.AppImage` : `chmod +x` puis double-clic |

### Compiler depuis les sources (Linux)

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
| `L` | Paroles en plein écran |
| `^` / `$` (AZERTY), `[` / `]` (QWERTY) | Ralentir / accélérer la lecture |
| `Alt` + `↑` / `↓` | Déplacer le morceau choisi dans la file (`Suppr` le retire) |
| `F` | Clip en plein écran (section Clips) |
| `Échap` | Fermer une fenêtre ou un panneau |
| Touches média | Lecture, pause, suivant, précédent |
| `Tab` | Déplacer le focus. Les barres de progression se règlent avec `←` / `→`. |

## Formats pris en charge

mp3, flac, m4a, aac, ogg, opus, wav, aiff, wma, ainsi que d'autres formats moins courants (wv, ape, mpc…).
Les conteneurs vidéo (mp4, webm, mkv, mov) sont lus pour leur son ; leur image s'affiche dans la section **Clips**.

Tu peux choisir les formats dans **Réglages → Formats acceptés**. Le décodage passe par GStreamer : un format peu courant peut demander un greffon (`gst-plugins-good`, `gst-libav`).

## Architecture

```
src/                 Interface : HTML, CSS et JavaScript, sans framework ni étape de build
  app.js             Affichage, listes virtualisées, réglages, lecteur
  logic.js           Règles de lecture (suivant, aléatoire, répétition), testées à part
src-tauri/src/       Cœur en Rust
  main.rs            Commandes Tauri, état partagé, démarrage
  library.rs         Analyse des dossiers, tags et pochettes (lofty)
  frames.rs          Pochette tirée d'une vidéo (Linux, GStreamer)
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

Ta musique et tes réglages restent sur ta machine : aucun compte, aucune statistique envoyée. Écho ne se connecte qu'à ces services :

- les mises à jour, sur GitHub (désactivables) ;
- les paroles en ligne, sur LRCLIB et lyrics.ovh, si tu les actives : seuls l'artiste, le titre et la durée sont envoyés ;
- les pochettes en ligne, sur MusicBrainz, le Cover Art Archive et Deezer, si tu les actives (ou si tu en cherches une par clic droit) : seuls l'artiste, le titre et l'album sont envoyés ;
- les photos des artistes, sur Deezer, si tu les actives : seul le nom de l'artiste est envoyé.

| Emplacement | Contenu |
|---|---|
| `~/.config/com.echo.player/` | Réglages, cache de la bibliothèque, corrections des morceaux |
| `~/.cache/com.echo.player/covers/` | Pochettes extraites des fichiers ou trouvées en ligne (peut être vidé depuis les réglages) |

Si un fichier de réglages est abîmé, il est mis de côté (sous le nom `settings.json.bad`) plutôt qu'effacé.

## Licence

Distribué sous licence [MIT](LICENSE).

## Statut Discord

Le statut « Écoute … » passe par l'application Discord **Écho** (identifiant déjà intégré dans `APP_ID`, `src-tauri/src/discord.rs`). Active **Réglages → Système → Statut Discord**, avec l'app Discord ouverte sur le même PC.

Pour utiliser ta propre application Discord, crée-la sur [discord.com/developers/applications](https://discord.com/developers/applications) et remplace `APP_ID`.

Pour essayer sans recompiler : lance Écho avec la variable `ECHO_DISCORD_APP_ID=<identifiant>`.
