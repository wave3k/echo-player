/* Écho · site
   Détecte le système, puis remplace les liens de téléchargement par ceux de la
   dernière version publiée. Si l'API GitHub ne répond pas, les liens de secours
   (version écrite dans le HTML) restent en place. */
(function () {
  "use strict";

  var REPO = "wave3k/echo-player";
  var API = "https://api.github.com/repos/" + REPO + "/releases/latest";
  var ASSETS = [
    { key: "exe", re: /x64-setup\.exe$/ },
    { key: "msi", re: /x64_en-US\.msi$/ },
    { key: "deb", re: /amd64\.deb$/ },
    { key: "appimage", re: /amd64\.AppImage$/ }
  ];

  function detectOs() {
    var ua = navigator.userAgent || "";
    if (/Windows/i.test(ua)) return "windows";
    if (/Android/i.test(ua)) return "android";
    if (/Linux/i.test(ua)) return "linux";
    if (/Mac/i.test(ua)) return "mac";
    return "other";
  }

  var os = detectOs();

  // Met en avant la carte du système du visiteur et adapte le bouton principal.
  var card = document.querySelector('.dl[data-os="' + os + '"]');
  if (card) {
    card.classList.add("is-detected");
    var first = card.querySelector(".btn");
    if (first) first.classList.remove("ghost");
  }
  var cta = document.getElementById("cta-main");
  if (cta) {
    if (os === "windows") cta.textContent = "Télécharger pour Windows";
    else if (os === "linux") cta.textContent = "Télécharger pour Linux";
  }

  function setVersion(tag) {
    var num = document.getElementById("dl-num");
    if (num && tag) num.textContent = tag.replace(/^v/, "");
  }

  function applyRelease(release) {
    if (!release || !release.assets) return;
    setVersion(release.tag_name);
    ASSETS.forEach(function (spec) {
      var asset = release.assets.filter(function (a) { return spec.re.test(a.name); })[0];
      if (!asset) return;
      document.querySelectorAll('[data-asset="' + spec.key + '"]').forEach(function (link) {
        link.setAttribute("href", asset.browser_download_url);
      });
    });
  }

  if (window.fetch && document.querySelector("[data-asset]")) {
    fetch(API, { headers: { Accept: "application/vnd.github+json" } })
      .then(function (r) { return r.ok ? r.json() : null; })
      .then(applyRelease)
      .catch(function () { /* liens de secours déjà présents */ });
  }
})();
