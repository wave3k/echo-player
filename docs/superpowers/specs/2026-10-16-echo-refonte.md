# Écho — refonte de l'interface (2026-10-16)

L'application s'appelait MusicApp ; elle devient **Écho**, avec une identité, trois dispositions
au choix et une vraie page de réglages. Le cœur (scan des dossiers, lecture, MPRIS) ne change pas.

## Identité

- Nom : **Écho**. Identifiant Tauri : `com.echo.player`, binaire `echo`.
- Logo : carré arrondi en dégradé bleu → violet, avec une onde sonore blanche
  (icône Hugeicons `waveform`). Décliné en 32, 128, 256 et 512 px, plus un `.ico`.
- Migration : au premier lancement, les dossiers de `~/.config/com.musicapp.player/`
  sont repris dans `~/.config/com.echo.player/`, ainsi que les pochettes en cache.

## Les trois dispositions

| Disposition | Principe | Détails |
| --- | --- | --- |
| **Normale** | Navigation classique | Barre latérale (Écouter, Bibliothèque, Vidéos, Réglages), contenu à droite, la file d'attente sous la vue Écouter, barre de lecture en bas. |
| **Compacte** | Pochettes en carrousel | Une pochette centrale (190 px) entourée des voisines (±138 px, bouclage sur la file), titre, avancement, cinq commandes, dock de six icônes en bas. Bibliothèque, vidéos, réglages et file s'ouvrent en panneaux superposés. |
| **Minimaliste** | L'album d'abord | Couverture à gauche (300 px) avec les albums voisins en tranches cliquables, fiche de l'album à droite (titre, artiste, titres), barre basse compacte. Une recherche filtre les titres ou fouille toute la bibliothèque. |

Le choix se fait dans Réglages → Disposition (vignettes cliquables) ; il est conservé d'une
session à l'autre.

## Apparence

- **Thème** : Système / Clair / Sombre.
- **Palettes** : Nuit bleue (défaut), Aurore, Menthe, Prune — chacune déclinée en clair et en sombre
  (8 blocs de variables CSS).
- **Polices** : Space Grotesk (défaut), Sora, Outfit — fichiers locaux, aucune requête réseau.
- **Taille du texte** : Petit / Normal / Grand (racine 15 / 16 / 17 px).
- **Coins arrondis** : curseur de 0 à 20 px ; les cercles (bouton lecture, interrupteurs) restent ronds.

## Page Réglages

Six sections : Apparence (thème, palette, police, disposition, animations, pochettes, arrondis,
taille du texte), Lecture (reprise, position, aléatoire, répétition, saut avant/arrière, passage
au suivant en cas d'erreur), Bibliothèque (dossiers, actualiser au démarrage), Formats acceptés,
Pochettes en cache (vider, taille), Système (touches média, démarrage automatique, confirmation
avant de quitter), Données (exporter / importer / réinitialiser) et À propos.

Quatre réglages sont annoncés mais désactivés avec la mention « bientôt » : fondu enchaîné,
volume égal, surveillance des changements de dossier, icône dans la barre du système,
notification par morceau.

## Reprise au lancement

Le morceau et la position sont mémorisés. Au démarrage, Écho restaure le titre et la position ;
la lecture ne repart que si elle était en cours à la fermeture. Une vidéo reprise sans lecture
n'ouvre pas sa vue : elle attend dans le lecteur, et un clic sur lecture l'ouvre.

## Notes techniques

- L'interface est **embarquée dans le binaire** (`--features custom-protocol`) : l'app installée
  ne dépend pas du dossier du projet.
- Les médias passent par le protocole d'asset de Tauri (`protocol-asset`), les pochettes par
  `get_cover` (base64).
- Raccourcis : `Espace`, `Maj + ←/→`, `←/→` (saut), `Échap`.
- Mesure mémoire (PSS, machine de développement) : **278 Mo** pour Écho contre ~455 Mo pour
  Spotify. Descendre sous 150 Mo supposerait une réécriture en GTK4 : décision reportée.
