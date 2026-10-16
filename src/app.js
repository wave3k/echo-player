const { invoke, convertFileSrc } = window.__TAURI__.core;
const { open } = window.__TAURI__.dialog;
const { listen } = window.__TAURI__.event;

const ROW_HEIGHT = 52;

const state = {
  folder: null,
  folderMissing: false,
  tracks: [],
  view: "listen",
  // file de lecture courante (ids de morceaux, dans l'ordre affiche)
  queue: [],
  queueIndex: -1,
  shuffle: false,
  repeat: "off", // off | all | one
  listOrder: [],
  search: "",
  sort: "title",
  seeking: false,
};

const coverCache = new Map();

const el = (id) => document.getElementById(id);

/* ---------------- utilitaires ---------------- */

function normalize(text) {
  return (text || "")
    .toLowerCase()
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "");
}

function formatTime(ms) {
  if (!ms || ms < 0) return "0:00";
  const total = Math.floor(ms / 1000);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const ss = String(s).padStart(2, "0");
  if (h > 0) return `${h}:${String(m).padStart(2, "0")}:${ss}`;
  return `${m}:${ss}`;
}

function trackById(id) {
  return state.tracks.find((t) => t.id === id) || null;
}

function hueFor(text) {
  let h = 0;
  for (let i = 0; i < text.length; i++) {
    h = (h * 31 + text.charCodeAt(i)) % 360;
  }
  return h;
}

function coverStyle(track) {
  if (track && track.cover && coverCache.has(track.cover)) {
    return `background-image:url("${coverCache.get(track.cover)}")`;
  }
  const hue = hueFor(track && track.id ? track.id : "x");
  return `background:linear-gradient(135deg, hsl(${hue} 45% 26%), hsl(${(hue + 40) % 360} 40% 16%))`;
}

async function loadCover(track, node) {
  if (!track || !track.cover) return;
  if (coverCache.has(track.cover)) {
    node.setAttribute("style", coverStyle(track));
    return;
  }
  try {
    const data = await invoke("get_cover", { id: track.cover });
    if (data) {
      coverCache.set(track.cover, data);
      node.setAttribute("style", coverStyle(track));
    }
  } catch (_) {
    /* pochette illisible : on garde le degrade */
  }
}

/* ---------------- liste affichee (recherche + tri) ---------------- */

function visibleTracks() {
  const q = normalize(state.search);
  // La bibliotheque sert la musique : les clips vidéo vivent dans leur vue.
  let list = state.tracks.filter((t) => !t.is_video);
  if (q) {
    list = list.filter(
      (t) =>
        normalize(t.title).includes(q) ||
        normalize(t.artist).includes(q) ||
        normalize(t.album).includes(q)
    );
  }
  const key = state.sort;
  const dir = key === "duration" ? -1 : 1;
  return [...list].sort((a, b) => {
    if (key === "duration") return (a.duration_ms - b.duration_ms) * dir;
    return String(a[key] || "").localeCompare(String(b[key] || ""), "fr", {
      sensitivity: "base",
    });
  });
}

/* ---------------- rendu ---------------- */

function renderAll() {
  const audios = state.tracks.filter((t) => !t.is_video);
  el("track-count").textContent =
    state.tracks.length === 0
      ? "Aucun morceau"
      : `${audios.length} morceau${audios.length > 1 ? "x" : ""} · ${
          state.tracks.length - audios.length
        } vidéo${state.tracks.length - audios.length > 1 ? "s" : ""}`;
  el("empty").hidden = !!state.folder;
  el("banner").hidden = !state.folderMissing;
  renderLibrary();
  renderVideos();
  renderQueue();
  renderNowPlaying();
}

function renderLibrary() {
  const list = el("list");
  const all = visibleTracks();
  state.listOrder = all.map((t) => t.id);

  const spacer = document.createElement("div");
  spacer.className = "list-spacer";
  spacer.style.height = `${all.length * ROW_HEIGHT}px`;
  list.replaceChildren(spacer);

  const draw = () => {
    const scrollTop = list.scrollTop;
    const start = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - 5);
    const end = Math.min(
      all.length,
      Math.ceil((scrollTop + list.clientHeight) / ROW_HEIGHT) + 5
    );
    const frag = document.createDocumentFragment();
    for (let i = start; i < end; i++) {
      const track = all[i];
      const row = document.createElement("div");
      row.className = "row" + (track.id === currentId() ? " is-current" : "");
      row.style.top = `${i * ROW_HEIGHT}px`;
      row.dataset.index = String(i);
      row.innerHTML = `
        <div class="row-index">${i + 1}</div>
        <div class="row-main">
          <div class="thumb art-${i}"></div>
          <div class="row-text">
            <div class="row-title"></div>
            <div class="row-artist"></div>
          </div>
        </div>
        <div class="row-album"></div>
        <div class="row-time">${formatTime(track.duration_ms)}</div>`;
      row.querySelector(".row-title").textContent = track.title;
      row.querySelector(".row-artist").textContent = track.artist || "—";
      row.querySelector(".row-album").textContent = track.album || "—";
      const thumb = row.querySelector(".thumb");
      thumb.setAttribute("style", coverStyle(track));
      loadCover(track, thumb);
      row.addEventListener("click", () => playFromList(i));
      frag.appendChild(row);
    }
    spacer.replaceChildren(...[]);
    spacer.appendChild(frag);
    spacer.style.height = `${all.length * ROW_HEIGHT}px`;
  };

  list.onscroll = draw;
  draw();
}

function renderVideos() {
  const tiles = el("tiles");
  const vids = state.tracks.filter((t) => t.is_video);
  tiles.replaceChildren();
  for (const track of vids) {
    const tile = document.createElement("button");
    tile.className = "tile";
    tile.innerHTML = `
      <div class="tile-art">
        <svg viewBox="0 0 24 24" class="icon icon-stroke"><circle cx="12" cy="12" r="8.5"/><path d="M10.2 9.2 15 12l-4.8 2.8Z" class="icon-fill"/></svg>
      </div>
      <div class="tile-body">
        <div class="tile-title"></div>
        <div class="tile-sub"></div>
      </div>`;
    tile.querySelector(".tile-title").textContent = track.title;
    tile.querySelector(".tile-sub").textContent =
      track.artist || formatTime(track.duration_ms);
    const art = tile.querySelector(".tile-art");
    art.setAttribute("style", coverStyle(track));
    loadCover(track, art);
    tile.addEventListener("click", () => playVideo(track));
    tiles.appendChild(tile);
  }
  if (vids.length === 0 && state.folder) {
    const p = document.createElement("div");
    p.className = "tile-sub";
    p.textContent = "Aucune vidéo trouvée dans ce dossier.";
    tiles.appendChild(p);
  }
}

function renderQueue() {
  const queue = el("queue");
  queue.replaceChildren();
  const ids = state.queue.length ? state.queue : [];
  ids.slice(0, 60).forEach((id, i) => {
    const track = trackById(id);
    if (!track) return;
    const item = document.createElement("button");
    item.className = "queue-item" + (id === currentId() ? " is-current" : "");
    item.innerHTML = `<span class="queue-num">${i + 1}</span>
      <span class="queue-name"></span>
      <span class="queue-time">${formatTime(track.duration_ms)}</span>`;
    item.querySelector(".queue-name").textContent =
      `${track.title}${track.artist ? " — " + track.artist : ""}`;
    item.addEventListener("click", () => {
      state.queueIndex = i;
      playCurrent();
    });
    queue.appendChild(item);
  });
  if (ids.length === 0) {
    const p = document.createElement("div");
    p.className = "count";
    p.textContent = "La file est vide.";
    queue.appendChild(p);
  }
}

function currentId() {
  return state.queue[state.queueIndex] || null;
}

function renderNowPlaying() {
  const track = trackById(currentId());
  const miniCover = el("mini-cover");
  const listenCover = el("listen-cover");
  if (!track) {
    el("mini-title").textContent = "Aucun morceau";
    el("mini-artist").textContent = "—";
    el("listen-title").textContent = "Rien en cours";
    el("listen-artist").textContent = "Choisis un morceau dans ta bibliothèque";
    miniCover.setAttribute("style", "");
    listenCover.setAttribute("style", "");
    return;
  }
  el("mini-title").textContent = track.title;
  el("mini-artist").textContent = track.artist || "—";
  el("listen-title").textContent = track.title;
  el("listen-artist").textContent = track.artist || "—";
  miniCover.setAttribute("style", coverStyle(track));
  listenCover.setAttribute("style", coverStyle(track));
  loadCover(track, miniCover);
  loadCover(track, listenCover);
  document.querySelectorAll(".row").forEach((row, i) => {
    row.classList.toggle("is-current", i === state.queueIndex && row.dataset.index === String(i));
  });
}

/* ---------------- touches media du systeme ---------------- */

let lastPositionPush = 0;

function pushNowPlaying(track) {
  if (!track) {
    invoke("set_now_playing", {
      title: "",
      artist: "",
      album: "",
      cover: null,
      durationMs: 0,
    }).catch(() => {});
    return;
  }
  invoke("set_now_playing", {
    title: track.title,
    artist: track.artist || "",
    album: track.album || "",
    cover: track.cover || null,
    durationMs: track.duration_ms || 0,
  }).catch(() => {});
}

function pushPlayback(force = false) {
  const track = trackById(currentId());
  const media = track && track.is_video ? video : audio;
  const now = Date.now();
  if (!force && now - lastPositionPush < 5000) return;
  lastPositionPush = now;
  invoke("set_playback", {
    playing: !media.paused && !!track,
    positionMs: Math.round((media.currentTime || 0) * 1000),
  }).catch(() => {});
}

async function bindMediaKeys() {
  await listen("media-control", (event) => {    const track = trackById(currentId());
    const media = track && track.is_video ? video : audio;
    switch (event.payload) {
      case "play":
        media.play().catch(() => {});
        break;
      case "pause":
        media.pause();
        break;
      case "toggle":
        togglePlay();
        break;
      case "stop":
        media.pause();
        media.currentTime = 0;
        break;
      case "next":
        step(1);
        break;
      case "previous":
        step(-1);
        break;
      case "seek-forward":
        media.currentTime = Math.min(media.duration || 0, media.currentTime + 10);
        break;
      case "seek-backward":
        media.currentTime = Math.max(0, media.currentTime - 10);
        break;
      default:
        break;
    }
    pushPlayback(true);
  });
}

/* ---------------- lecture ---------------- */

const audio = el("audio");
const video = el("video");

function playFromList(index) {
  state.queue = [...state.listOrder];
  state.queueIndex = index;
  const id = state.queue[index];
  const track = trackById(id);
  if (track && track.is_video) {
    switchView("videos");
    playVideo(track);
    return;
  }
  playCurrent();
}

function playVideo(track) {
  audio.pause();
  switchView("videos");
  el("video-wrap").hidden = false;
  video.src = convertFileSrc(track.path);
  video.play().catch(() => {});
  state.queue = state.tracks.filter((t) => t.is_video).map((t) => t.id);
  state.queueIndex = state.queue.indexOf(track.id);
  pushNowPlaying(track);
  pushPlayback(true);
  renderNowPlaying();
  renderQueue();
}

function playCurrent() {
  const track = trackById(currentId());
  if (!track) return;
  if (track.is_video) {
    playVideo(track);
    return;
  }
  el("video-wrap").hidden = true;
  video.pause();
  audio.src = convertFileSrc(track.path);
  audio.play().catch(() => {});
  pushNowPlaying(track);
  pushPlayback(true);
  renderNowPlaying();
  renderQueue();
}

function togglePlay() {
  const track = trackById(currentId());
  if (!track) {
    if (state.listOrder.length) playFromList(0);
    return;
  }
  const media = track.is_video ? video : audio;
  if (media.paused) media.play().catch(() => {});
  else media.pause();
}

function nextIndex(auto) {
  const n = state.queue.length;
  if (n === 0) return -1;
  if (state.shuffle && n > 1) {
    let i = state.queueIndex;
    while (i === state.queueIndex) i = Math.floor(Math.random() * n);
    return i;
  }
  if (state.queueIndex + 1 < n) return state.queueIndex + 1;
  if (auto && state.repeat === "one") return state.queueIndex;
  if (auto && state.repeat === "all") return 0;
  if (auto) return -1;
  return 0;
}

function step(delta, auto = false) {
  if (state.queue.length === 0) {
    if (state.listOrder.length) {
      state.queue = [...state.listOrder];
      state.queueIndex = 0;
      playCurrent();
    }
    return;
  }
  if (delta > 0) {
    const i = nextIndex(auto);
    if (i < 0) {
      audio.pause();
      video.pause();
      return;
    }
    state.queueIndex = i;
  } else {
    state.queueIndex =
      state.queueIndex - 1 < 0 ? state.queue.length - 1 : state.queueIndex - 1;
  }
  playCurrent();
}

function updateProgress() {
  const track = trackById(currentId());
  const media = track && track.is_video ? video : audio;
  const dur = media.duration || 0;
  if (!state.seeking && isFinite(dur) && dur > 0) {
    el("seek").value = String(Math.round((media.currentTime / dur) * 1000));
  }
  el("time-now").textContent = formatTime(media.currentTime * 1000);
  el("time-total").textContent = formatTime((dur || 0) * 1000);
  const playing = !media.paused;
  el("btn-play").classList.toggle("is-playing", playing);
  el("btn-play-big").classList.toggle("is-playing", playing);
  if (playing) pushPlayback();
}

/* ---------------- vues ---------------- */

function switchView(view) {
  state.view = view;
  document
    .querySelectorAll(".nav-item")
    .forEach((b) => b.classList.toggle("is-active", b.dataset.view === view));
  document
    .querySelectorAll(".view")
    .forEach((v) => v.classList.toggle("is-active", v.id === `view-${view}`));
  if (view === "library") renderLibrary();
}

/* ---------------- dossiers ---------------- */

async function chooseFolder() {
  try {
    const picked = await open({ directory: true, multiple: false, title: "Choisir le dossier de musique" });
    if (!picked) return;
    const path = typeof picked === "string" ? picked : picked.path;
    const snapshot = await invoke("set_folder", { path });
    applySnapshot(snapshot);
  } catch (err) {
    console.error(err);
  }
}

async function rescan() {
  if (!state.folder) {
    chooseFolder();
    return;
  }
  const snapshot = await invoke("rescan");
  applySnapshot(snapshot);
}

function applySnapshot(snapshot) {
  state.folder = snapshot.folder;
  state.folderMissing = snapshot.folder_missing;
  state.tracks = snapshot.tracks;
  el("empty").hidden = !!state.folder;
  el("banner").hidden = !state.folderMissing;
  renderAll();
}

/* ---------------- branchements ---------------- */

document.querySelectorAll(".nav-item").forEach((btn) => {
  btn.addEventListener("click", () => switchView(btn.dataset.view));
});

el("search").addEventListener("input", (e) => {
  state.search = e.target.value;
  renderLibrary();
});

el("sort").addEventListener("change", (e) => {
  state.sort = e.target.value;
  renderLibrary();
});

el("btn-folder").addEventListener("click", chooseFolder);
el("btn-empty-folder").addEventListener("click", chooseFolder);
el("btn-banner-folder").addEventListener("click", chooseFolder);
el("btn-banner-close").addEventListener("click", () => (el("banner").hidden = true));
el("btn-rescan").addEventListener("click", rescan);
el("btn-play").addEventListener("click", togglePlay);
el("btn-play-big").addEventListener("click", togglePlay);
el("btn-next").addEventListener("click", () => step(1));
el("btn-prev").addEventListener("click", () => step(-1));

el("btn-shuffle").addEventListener("click", () => {
  state.shuffle = !state.shuffle;
  el("btn-shuffle").classList.toggle("is-on", state.shuffle);
});

el("btn-repeat").addEventListener("click", () => {
  const order = ["off", "all", "one"];
  state.repeat = order[(order.indexOf(state.repeat) + 1) % 3];
  el("btn-repeat").classList.toggle("is-on", state.repeat !== "off");
  el("btn-repeat").classList.toggle("is-one", state.repeat === "one");
});

el("btn-mute").addEventListener("click", () => {
  audio.muted = !audio.muted;
  video.muted = audio.muted;
  el("btn-mute").classList.toggle("is-on", audio.muted);
});

el("volume").addEventListener("input", (e) => {
  const v = Number(e.target.value) / 100;
  audio.volume = v;
  video.volume = v;
});

el("seek").addEventListener("input", (e) => {
  state.seeking = true;
  const track = trackById(currentId());
  const media = track && track.is_video ? video : audio;
  const dur = media.duration || 0;
  el("time-now").textContent = formatTime((Number(e.target.value) / 1000) * dur * 1000);
});

el("seek").addEventListener("change", (e) => {
  const track = trackById(currentId());
  const media = track && track.is_video ? video : audio;
  const dur = media.duration || 0;
  if (dur > 0) media.currentTime = (Number(e.target.value) / 1000) * dur;
  state.seeking = false;
});

[audio, video].forEach((media) => {
  media.addEventListener("timeupdate", updateProgress);
  media.addEventListener("loadedmetadata", updateProgress);
  media.addEventListener("play", () => {
    updateProgress();
    pushPlayback(true);
  });
  media.addEventListener("pause", () => {
    updateProgress();
    pushPlayback(true);
  });
  media.addEventListener("ended", () => step(1, true));
});

document.addEventListener("keydown", (e) => {
  if (e.target.tagName === "INPUT" || e.target.tagName === "SELECT") return;
  if (e.code === "Space") {
    e.preventDefault();
    togglePlay();
  } else if (e.code === "ArrowRight" && e.shiftKey) {
    step(1);
  } else if (e.code === "ArrowLeft" && e.shiftKey) {
    step(-1);
  }
});

async function bindBackendEvents() {
  await listen("library-updated", (event) => {
    applySnapshot(event.payload);
  });
  await listen("scanning", (event) => {
    if (event.payload) {
      el("track-count").textContent = "Analyse du dossier…";
    } else {
      const audios = state.tracks.filter((t) => !t.is_video).length;
      const videos = state.tracks.length - audios;
      el("track-count").textContent = `${audios} morceau${audios > 1 ? "x" : ""} · ${videos} vidéo${videos > 1 ? "s" : ""}`;
    }
  });
}

/* ---------------- demarrage ---------------- */

(async function start() {
  const snapshot = await invoke("get_state");
  applySnapshot(snapshot);
  bindMediaKeys().catch(() => {});
  bindBackendEvents().catch(() => {});
})();
