/** @type {import('tailwindcss').Config} */
module.exports = {
  content: [
    "./src/**/*.rs",
    "./static/**/*.html",
    "./static/**/*.css",
    "./templates/**/*.html",
  ],
  // The @import "tailwindcss" in static/tailwind.css resolves to this package
  // when running via npx from the project root.
  plugins: [],
}