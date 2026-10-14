// Show / hide the password fields on the auth forms.
//
// A separate file rather than an inline <script> because the CSP is
// `script-src 'self'`, which blocks inline scripts. Same reason htmx is
// vendored into static/vendor/ instead of loaded from a CDN.
//
// One delegated listener on the document rather than one per button: the form
// is re-rendered by the server on a validation error, and a listener bound
// directly to a button would be lost with it.
//
// Add `data-password-toggle` to a button inside a <form> and every password
// input in that form is toggled together.

document.addEventListener("click", function (event) {
  const button = event.target.closest("[data-password-toggle]");
  if (!button) {
    return;
  }

  const form = button.closest("form");
  if (!form) {
    return;
  }

  const fields = form.querySelectorAll('input[type="password"]');
  // Decide from the first field, then apply the same answer to all of them —
  // otherwise a confirm field on a different state would drift out of sync.
  const willShow = fields.length > 0 && fields[0].type === "password";

  fields.forEach(function (field) {
    field.type = willShow ? "text" : "password";
  });

  button.textContent = willShow ? "Hide" : "Show";
  button.setAttribute("aria-pressed", String(willShow));
});