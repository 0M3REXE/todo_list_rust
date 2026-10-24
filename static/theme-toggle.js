// Theme toggle — light mode default, persists preference in localStorage,
// applies `data-theme` attribute on <html> so CSS can switch palettes.

(function () {
  var html = document.documentElement;
  var toggle = document.querySelector('.theme-toggle');
  var icon = document.querySelector('.theme-toggle-icon');

  if (!toggle || !icon) return;

  var THEME_KEY = 'todo-theme';
  var DARK = 'dark';
  var LIGHT = 'light';

  var ICON_DARK = '☀️';   // light mode active → show sun (switch to light)
  var ICON_LIGHT = '🌙';  // dark mode active → show moon (switch to dark)

  function setTheme(theme) {
    if (theme === DARK) {
      html.setAttribute('data-theme', DARK);
      localStorage.setItem(THEME_KEY, DARK);
      icon.textContent = ICON_DARK;
      toggle.setAttribute('aria-pressed', 'true');
    } else {
      html.removeAttribute('data-theme');
      localStorage.setItem(THEME_KEY, LIGHT);
      icon.textContent = ICON_LIGHT;
      toggle.setAttribute('aria-pressed', 'false');
    }
  }

  // Initial theme: saved preference, else light. Never follow system.
  var saved = localStorage.getItem(THEME_KEY);
  setTheme(saved ? saved : LIGHT);

  toggle.addEventListener('click', function () {
    var isDark = html.getAttribute('data-theme') === DARK;
    setTheme(isDark ? LIGHT : DARK);
  });
})();
