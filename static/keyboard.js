// Keyboard shortcuts + client-side todo search.
// Shortcuts:
//   /  → focus search box
//   n  → focus new-todo input
//   esc → clear search / blur active input

(function () {
  var searchInput = document.getElementById('todo-search');
  var newTodoInput = document.getElementById('todo-text');
  var rows = document.querySelectorAll('.todo-row[data-text]');

  // --- Search filter -------------------------------------------------------

  function filterTodos() {
    var q = (searchInput ? searchInput.value : '').toLowerCase().trim();
    var visible = 0;

    rows.forEach(function (row) {
      var text = (row.getAttribute('data-text') || '').toLowerCase();
      var match = q === '' || text.indexOf(q) !== -1;
      row.style.display = match ? '' : 'none';
      if (match) visible++;
    });

    var empty = document.querySelector('.todo-empty');
    if (empty) {
      empty.style.display = visible === 0 && q !== '' ? '' : 'none';
    }
  }

  if (searchInput) {
    searchInput.addEventListener('input', filterTodos);
  }

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
      if (searchInput) {
        searchInput.focus();
        searchInput.select();
      }
      return;
    }

    if (event.key === 'n' && !isTyping && !event.ctrlKey && !event.metaKey) {
      event.preventDefault();
      if (newTodoInput) {
        newTodoInput.focus();
      }
      return;
    }

    if (event.key === 'Escape') {
      if (searchInput && document.activeElement === searchInput) {
        searchInput.value = '';
        filterTodos();
        searchInput.blur();
      }
      return;
    }
  });

  // Refresh row references after htmx swaps the list fragment
  document.addEventListener('htmx:afterSwap', function (event) {
    if (event.target && event.target.id === 'todo-list') {
      rows = document.querySelectorAll('.todo-row[data-text]');
    }
  });
})();
