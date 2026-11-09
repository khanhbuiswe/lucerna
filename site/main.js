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
