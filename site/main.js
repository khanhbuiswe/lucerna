document.querySelectorAll('.copy').forEach(function (btn) {
  btn.addEventListener('click', function () {
    var text = btn.previousElementSibling.textContent;
    var done = function () { btn.textContent = 'Copied'; setTimeout(function () { btn.textContent = 'Copy'; }, 1400); };
    try {
      navigator.clipboard.writeText(text).then(done, function () { selectText(btn.previousElementSibling); });
    } catch (e) { selectText(btn.previousElementSibling); }
  });
});
function selectText(el) { var r = document.createRange(); r.selectNodeContents(el); var s = getSelection(); s.removeAllRanges(); s.addRange(r); }

// Download buttons link to the release page. On a Mac with Apple silicon,
// swap in the disk image from data-dmg; anything else, or a browser that
// will not say, keeps the release page.
(function () {
  var links = document.querySelectorAll('a[data-dmg]');
  if (!links.length) return;
  var useDmg = function () { links.forEach(function (a) { a.href = a.getAttribute('data-dmg'); }); };
  var uad = navigator.userAgentData;
  if (uad && uad.getHighEntropyValues) {
    // Chromium browsers report the CPU architecture directly.
    if (uad.platform !== 'macOS') return;
    uad.getHighEntropyValues(['architecture']).then(function (v) {
      if (v.architecture === 'arm') useDmg();
    }, function () {});
    return;
  }
  // Safari and Firefox say "Macintosh" on every Mac, and iPadOS Safari does
  // too (it has touch points), so look at the GPU instead.
  if (!/Macintosh/.test(navigator.userAgent) || navigator.maxTouchPoints > 1) return;
  if (appleSiliconGpu()) useDmg();
})();

// True when WebGL shows an Apple silicon GPU: a renderer named "Apple M…",
// or Safari's masked "Apple GPU" together with ASTC textures, which Intel
// Macs lack.
function appleSiliconGpu() {
  try {
    var gl = document.createElement('canvas').getContext('webgl');
    if (!gl) return false;
    var info = gl.getExtension('WEBGL_debug_renderer_info');
    var renderer = String(gl.getParameter(info ? info.UNMASKED_RENDERER_WEBGL : gl.RENDERER));
    if (/Apple M\d/.test(renderer)) return true;
    return /Apple GPU/.test(renderer) && !!gl.getExtension('WEBGL_compressed_texture_astc');
  } catch (e) {
    return false;
  }
}
