// Keyboard shortcuts + client-side todo search.
// Shortcuts:
//   /  → focus search box
//   n  → focus new-todo input
//   esc → clear search / blur active input

(function () {
  // --- Search filter -------------------------------------------------------

  function filterTodos() {
    var searchInput = document.getElementById('todo-search');
    var rows = document.querySelectorAll('.todo-row[data-text]');
    var q = (searchInput ? searchInput.value : '').toLowerCase().trim();
    var visible = 0;

    rows.forEach(function (row) {
      var text = (row.getAttribute('data-text') || '').toLowerCase();
      var match = q === '' || text.indexOf(q) !== -1;
      row.hidden = !match;
      if (match) visible++;
    });

    var emptyList = document.querySelector('[data-empty-list]');
    var noMatches = document.querySelector('[data-search-empty]');
    if (emptyList) emptyList.hidden = q !== '';
    if (noMatches) noMatches.hidden = q === '' || visible > 0;
  }

  document.addEventListener('input', function (event) {
    if (event.target && event.target.id === 'todo-search') filterTodos();
  });

  document.addEventListener('click', function (event) {
    var clearButton = event.target.closest && event.target.closest('[data-clear-search]');
    var searchInput = document.getElementById('todo-search');
    if (!clearButton || !searchInput) return;
    searchInput.value = '';
    filterTodos();
    searchInput.focus();
  });

  // --- Keyboard shortcuts --------------------------------------------------

  document.addEventListener('keydown', function (event) {
    // Ignore shortcuts when typing inside an input/textarea/select
    var target = event.target;
    var isTyping =
      target.tagName === 'INPUT' ||
      target.tagName === 'TEXTAREA' ||
      target.isContentEditable;

    if (event.key === '/' && !isTyping) {
      event.preventDefault();
      var searchInput = document.getElementById('todo-search');
      if (searchInput) {
        searchInput.focus();
        searchInput.select();
      }
      return;
    }

    if (event.key === 'n' && !isTyping && !event.ctrlKey && !event.metaKey) {
      event.preventDefault();
      var newTodoInput = document.getElementById('todo-text');
      if (newTodoInput) {
        newTodoInput.focus();
      }
      return;
    }

    if (event.key === 'Escape') {
      var searchInput = document.getElementById('todo-search');
      if (searchInput && document.activeElement === searchInput) {
        searchInput.value = '';
        filterTodos();
        searchInput.blur();
      }
      return;
    }
  });

  document.addEventListener('htmx:afterSwap', function (event) {
    if (event.target && event.target.id === 'todo-list') filterTodos();
  });

})();
