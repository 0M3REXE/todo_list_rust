// Theme toggle — persists preference in localStorage, respects
// prefers-color-scheme as the default, and applies a `data-theme`
// attribute on <html> so CSS can switch palettes.

(function () {
  var html = document.documentElement;
  var toggle = document.querySelector('.theme-toggle');
  var icon = document.querySelector('.theme-toggle-icon');

  if (!toggle || !icon) return;

  var THEME_KEY = 'todo-theme';
  var DARK = 'dark';
  var LIGHT = 'light';

  // Icons
  var ICON_DARK = '☀️';   // light mode is active → show sun (switch to light)
  var ICON_LIGHT = '🌙';  // dark mode is active → show moon (switch to dark)

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

  function getSystemTheme() {
    return window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches
      ? DARK
      : LIGHT;
  }

  // Initial theme: saved preference, else system, else light.
  var saved = localStorage.getItem(THEME_KEY);
  setTheme(saved ? saved : getSystemTheme());

  // Listen for system theme changes only if the user hasn't saved a preference.
  if (!saved) {
    window
      .matchMedia('(prefers-color-scheme: dark)')
      .addEventListener('change', function (e) {
        setTheme(e.matches ? DARK : LIGHT);
      });
  }

  toggle.addEventListener('click', function () {
    var isDark = html.getAttribute('data-theme') === DARK;
    setTheme(isDark ? LIGHT : DARK);
  });
})();
