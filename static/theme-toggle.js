// Theme preference and service-worker registration. Icons are inline SVGs,
// so no icon library or emoji font is required.
(function () {
  if ('serviceWorker' in navigator) {
    navigator.serviceWorker.register('/service-worker.js').catch(function (error) {
      console.warn('Service worker registration failed', error);
    });
    navigator.serviceWorker.getRegistrations().then(function (registrations) {
      registrations.forEach(function (registration) {
        if (new URL(registration.scope).pathname === '/static/') registration.unregister();
      });
    });
  }

  var html = document.documentElement;
  var toggle = document.querySelector('.theme-toggle');
  if (!toggle) return;

  var THEME_KEY = 'todo-theme';
  function setTheme(theme, persist) {
    var dark = theme === 'dark';
    if (dark) html.setAttribute('data-theme', 'dark');
    else html.removeAttribute('data-theme');

    toggle.setAttribute('aria-pressed', String(dark));
    var label = dark ? 'Switch to light mode' : 'Switch to dark mode';
    toggle.setAttribute('aria-label', label);
    toggle.setAttribute('title', label);

    if (persist) {
      try { localStorage.setItem(THEME_KEY, dark ? 'dark' : 'light'); }
      catch (_) { /* Theme still works for this page when storage is disabled. */ }
    }
  }

  var saved = null;
  try { saved = localStorage.getItem(THEME_KEY); }
  catch (_) { /* Use the light theme when storage is disabled. */ }
  setTheme(saved === 'dark' ? 'dark' : 'light', false);

  toggle.addEventListener('click', function () {
    var dark = html.getAttribute('data-theme') === 'dark';
    setTheme(dark ? 'light' : 'dark', true);
  });
})();
