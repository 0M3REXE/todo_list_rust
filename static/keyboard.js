// Keyboard shortcuts + client-side todo search.
// Shortcuts:
//   /  → focus search box
//   n  → focus new-todo input
//   esc → clear search / blur active input

(function () {
  var pendingListFocus = null;
  var searchQuery = '';

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
    if (event.target && event.target.id === 'todo-search') {
      searchQuery = event.target.value;
      filterTodos();
    }
    if (event.target && event.target.id === 'todo-text') validateTodoLength(event.target);
  });

  function validateTodoLength(input) {
    var limit = Number(input.dataset.maxlength);
    input.setCustomValidity(Array.from(input.value).length > limit
      ? 'Use ' + limit + ' characters or fewer.'
      : '');
  }

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

    if (event.key === '/' && !isTyping && !event.ctrlKey && !event.metaKey && !event.altKey) {
      event.preventDefault();
      var searchInput = document.getElementById('todo-search');
      if (searchInput) {
        searchInput.focus();
        searchInput.select();
      }
      return;
    }

    if (event.key === 'n' && !isTyping && !event.ctrlKey && !event.metaKey && !event.altKey) {
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

  document.addEventListener('htmx:before:request', function (event) {
    var ctx = event.detail && event.detail.ctx;
    var source = ctx && ctx.sourceElement;
    if (source && source.closest('.todo-row')) {
      var row = source.closest('.todo-row');
      var visibleRows = Array.from(document.querySelectorAll('.todo-row')).filter(function (item) {
        return !item.hidden;
      });
      pendingListFocus = {
        index: visibleRows.indexOf(row),
        request: source.getAttribute('hx-post'),
        action: source.classList.contains('todo-delete') ? 'delete' : 'toggle'
      };
    }
  });

  document.addEventListener('htmx:after:swap', function (event) {
    var ctx = event.detail && event.detail.ctx;
    var target = ctx && ctx.target;
    if (target && target.id === 'todo-panel') {
      var searchInput = document.getElementById('todo-search');
      if (searchInput) {
        searchInput.value = searchQuery;
        filterTodos();
      }
    }
    if (target && target.id === 'todo-list') {
      filterTodos();
      if (pendingListFocus) {
        var rows = Array.from(document.querySelectorAll('.todo-row'));
        if (pendingListFocus.action === 'toggle') {
          rows.map(function (row) { return row.querySelector('.todo-toggle'); })
            .find(function (control) {
              return control && control.getAttribute('hx-post') === pendingListFocus.request;
            })?.focus();
        } else {
          var visibleRows = rows.filter(function (row) { return !row.hidden; });
          if (visibleRows.length) visibleRows[Math.min(pendingListFocus.index, visibleRows.length - 1)]
            .querySelector('.todo-delete')?.focus();
          else if (searchQuery) document.getElementById('todo-search')?.focus();
          else document.getElementById('todo-text')?.focus();
        }
        pendingListFocus = null;
      }
    }
  });

})();
