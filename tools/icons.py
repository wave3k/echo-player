#!/usr/bin/env python3
"""Fabrique src/icons.js a partir des icones Hugeicons (licence MIT)."""
import json
import pathlib

SRC = pathlib.Path("/tmp/opencode/design/icons")
OUT = pathlib.Path("/home/teroryx/Projects/MusicApp/src/icons.js")

# nom utilise dans le code : fichier Hugeicons
WANTED = {
    "play": "PlayIcon",
    "pause": "PauseIcon",
    "stop": "StopIcon",
    "prev": "PreviousIcon",
    "next": "NextIcon",
    "shuffle": "ShuffleIcon",
    "repeat": "RepeatIcon",
    "volume_high": "VolumeHighIcon",
    "volume_low": "VolumeLowIcon",
    "volume_mute": "VolumeMute02Icon",
    "search": "Search01Icon",
    "close": "Cancel01Icon",
    "home": "Home01Icon",
    "library": "LibraryIcon",
    "video": "Video01Icon",
    "settings": "Settings01Icon",
    "heart": "HeartIcon",
    "more": "MoreHorizontalIcon",
    "queue": "Queue01Icon",
    "clock": "Clock01Icon",
    "note": "MusicNote01Icon",
    "note3": "MusicNote03Icon",
    "note4": "MusicNote04Icon",
    "note_square": "MusicNoteSquare01Icon",
    "disc": "DiscIcon",
    "album": "Album01Icon",
    "playlist": "Playlist01Icon",
    "headphones": "HeadphonesIcon",
    "waveform": "AudioWaveformIcon",
    "sun": "Sun01Icon",
    "moon": "Moon01Icon",
    "folder": "Folder01Icon",
    "refresh": "RefreshIcon",
    "plus": "Add01Icon",
    "check": "CheckmarkCircle01Icon",
    "palette": "PaletteIcon",
    "sliders": "SlidersHorizontalIcon",
    "sliders_v": "SlidersVerticalIcon",
    "history": "HistoryIcon",
    "dashboard": "DashboardCircleIcon",
    "sparkles": "SparklesIcon",
    "magic": "MagicWand01Icon",
    "download": "Download01Icon",
    "share": "Share01Icon",
    "star": "StarIcon",
    "star_half": "Star05Icon",
    "arrow_down": "ArrowDown01Icon",
    "arrow_right": "ArrowRight01Icon",
    "arrow_left": "ArrowLeft01Icon",
    "list": "ListViewIcon",
    "grid": "Grid02Icon",
    "filter": "FilterIcon",
    "chart": "ChartHistogramIcon",
    "guitar": "GuitarIcon",
    "drum": "DrumIcon",
    "mic": "Mic01Icon",
    "calendar": "CalendarFoldIcon",
    "user": "User02Icon",
    "maximize": "Maximize01Icon",
    "minimize": "Minimize01Icon",
}

entries = []
missing = []
for name, file in WANTED.items():
    path = SRC / f"{file}.svg"
    if not path.exists():
        missing.append(file)
        continue
    svg = path.read_text().strip()
    svg = svg.replace("\n", "")
    entries.append(f"  {name}: {json.dumps(svg)}")

if missing:
    raise SystemExit("icones introuvables : " + ", ".join(missing))

js = (
    "/* Icônes Hugeicons (licence MIT) — https://hugeicons.com\n"
    "   Fichier fabriqué par tools/icons ; chaque icône suit la couleur du texte. */\n"
    "const ICONS = {\n" + ",\n".join(entries) + "\n};\n\n"
    "function icon(name, size = 16) {\n"
    "  return `<span class=\"ic\" style=\"width:${size}px;height:${size}px\">${ICONS[name] || \"\"}</span>`;\n"
    "}\n\n"
    "window.ICONS = ICONS;\nwindow.icon = icon;\n"
)

OUT.write_text(js)
print("icones ecrites :", OUT, len(js) // 1024, "Ko,", len(entries), "icones")
