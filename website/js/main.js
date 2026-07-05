/* Ofox Desktop landing page — 增强脚本
   全部防御式：任何一步失败都不影响 HTML 静态兜底内容 */

(function () {
  "use strict";

  /* ── 1. latest.json 动态版本号 / 下载链 ─────────────────── */
  (function fetchLatest() {
    var supportsTimeout = typeof AbortSignal !== "undefined" && "timeout" in AbortSignal;
    fetch("https://desktop.ofox.ai/latest.json", {
      signal: supportsTimeout ? AbortSignal.timeout(4000) : undefined,
    })
      .then(function (res) {
        if (!res.ok) throw new Error("HTTP " + res.status);
        return res.json();
      })
      .then(function (data) {
        if (!data || typeof data.version !== "string") return;

        document.querySelectorAll("[data-version]").forEach(function (el) {
          el.textContent = "v" + data.version;
        });

        var dmg = data.downloads && data.downloads["darwin-aarch64"];
        if (typeof dmg === "string" && dmg.indexOf("https://") === 0) {
          document.querySelectorAll("[data-dmg-url]").forEach(function (el) {
            el.href = dmg;
          });
        }

        var dateEl = document.querySelector("[data-pubdate]");
        if (dateEl && data.pubDate) {
          var d = new Date(data.pubDate);
          if (!isNaN(d)) {
            dateEl.textContent =
              "· 发布于 " + d.getFullYear() + "-" +
              String(d.getMonth() + 1).padStart(2, "0") + "-" +
              String(d.getDate()).padStart(2, "0");
            dateEl.hidden = false;
          }
        }
      })
      .catch(function () {
        /* CORS / 超时 / 字段缺失：静默，HTML 兜底生效 */
      });
  })();

  /* ── 2. 滚动 reveal ─────────────────────────────────────── */
  (function scrollReveal() {
    var els = document.querySelectorAll(".reveal");
    if (!("IntersectionObserver" in window)) {
      els.forEach(function (el) { el.classList.add("in"); });
      return;
    }
    var io = new IntersectionObserver(
      function (entries) {
        entries.forEach(function (entry) {
          if (entry.isIntersecting) {
            entry.target.classList.add("in");
            io.unobserve(entry.target);
          }
        });
      },
      { threshold: 0.15, rootMargin: "0px 0px -5% 0px" }
    );
    els.forEach(function (el) { io.observe(el); });
  })();

  /* ── 3. 导航滚动态 ──────────────────────────────────────── */
  (function navState() {
    var nav = document.getElementById("nav");
    if (!nav) return;
    var ticking = false;
    function update() {
      nav.classList.toggle("scrolled", window.scrollY > 8);
      ticking = false;
    }
    window.addEventListener("scroll", function () {
      if (!ticking) {
        ticking = true;
        requestAnimationFrame(update);
      }
    }, { passive: true });
    update();
  })();
})();
