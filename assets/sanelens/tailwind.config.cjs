/** @type {import('tailwindcss').Config} */
module.exports = {
  content: ["./index.html", "./src/**/*.{svelte,ts}"],
  theme: {
    extend: {
      colors: {
        bg: "rgb(var(--bg-rgb) / <alpha-value>)",
        bg2: "rgb(var(--bg-2-rgb) / <alpha-value>)",
        panel: "rgb(var(--panel-rgb) / <alpha-value>)",
        panel2: "rgb(var(--panel-2-rgb) / <alpha-value>)",
        ink: "rgb(var(--ink-rgb) / <alpha-value>)",
        muted: "rgb(var(--muted-rgb) / <alpha-value>)",
        border: "rgb(var(--border-rgb) / <alpha-value>)",
        accent: "rgb(var(--accent-rgb) / <alpha-value>)",
        accent2: "rgb(var(--accent-2-rgb) / <alpha-value>)",
        accent3: "rgb(var(--accent-3-rgb) / <alpha-value>)",
      },
      boxShadow: {
        panel: "var(--shadow)",
      },
      fontFamily: {
        ui: [
          "Sora",
          "Avenir Next",
          "Futura",
          "Trebuchet MS",
          "Gill Sans",
          "sans-serif",
        ],
        mono: [
          "Iosevka",
          "JetBrains Mono",
          "SF Mono",
          "Menlo",
          "Monaco",
          "Consolas",
          "Liberation Mono",
          "monospace",
        ],
      },
      keyframes: {
        liftIn: {
          "0%": { opacity: "0", transform: "translateY(8px)" },
          "100%": { opacity: "1", transform: "translateY(0)" },
        },
        fadeIn: {
          "0%": { opacity: "0", transform: "translateY(3px)" },
          "100%": { opacity: "1", transform: "translateY(0)" },
        },
      },
      animation: {
        liftIn: "liftIn 0.4s ease both",
        fadeIn: "fadeIn 0.12s ease both",
      },
    },
  },
  plugins: [],
};
