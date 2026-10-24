// Applies the saved theme before the stylesheet paints.
//
// The toggle script (theme-toggle.js) is deferred, so dark-mode visitors
// would otherwise see a light flash on every page load. CSP blocks inline
// scripts, so this tiny file is loaded render-blocking (no `defer`) at the
// top of <head>.
(function () {
  try {
    var theme = localStorage.getItem('todo-theme');
    if (theme === 'dark') {
      document.documentElement.setAttribute('data-theme', 'dark');
    }
  } catch (_) { /* Storage disabled: the light default below is fine. */ }
})();
