/** @type {import('tailwindcss').Config} */

// magpie's tokens are RGB channels (src/index.css) so opacity modifiers work.
const channel = (name) => `rgb(var(--${name}) / <alpha-value>)`;

module.exports = {
  content: ["./src/index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  darkMode: ["selector", ".dark"],
  theme: {
    extend: {
      colors: {
        background: "hsl(var(--background))",
        foreground: "hsl(var(--foreground))",
        card: {
          DEFAULT: "hsl(var(--card))",
          foreground: "hsl(var(--card-foreground))",
        },
        popover: {
          DEFAULT: "hsl(var(--popover))",
          foreground: "hsl(var(--popover-foreground))",
        },
        primary: {
          DEFAULT: "hsl(var(--primary))",
          foreground: "hsl(var(--primary-foreground))",
        },
        secondary: {
          DEFAULT: "hsl(var(--secondary))",
          foreground: "hsl(var(--secondary-foreground))",
        },
        muted: {
          DEFAULT: "hsl(var(--muted))",
          foreground: "hsl(var(--muted-foreground))",
        },
        accent: {
          DEFAULT: "hsl(var(--accent))",
          foreground: "hsl(var(--accent-foreground))",
        },
        destructive: {
          DEFAULT: "hsl(var(--destructive))",
          foreground: "hsl(var(--destructive-foreground))",
        },
        border: "hsl(var(--border))",
        input: "hsl(var(--input))",
        ring: "hsl(var(--ring))",
        canvas: channel("canvas"),
        "card-2": channel("card-2"),
        pill: channel("pill"),
        "pill-hover": channel("pill-hover"),
        line: channel("line"),
        "line-2": channel("line-2"),
        ink: {
          DEFAULT: channel("ink"),
          2: channel("ink-2"),
          muted: channel("ink-muted"),
          faint: channel("ink-faint"),
          ctl: channel("ink-ctl"),
        },
        brand: {
          DEFAULT: channel("brand"),
          ink: channel("brand-ink"),
          soft: channel("brand-soft"),
          on: channel("on-brand"),
        },
        sel: channel("sel"),
        ok: {
          DEFAULT: channel("ok"),
          soft: channel("ok-soft"),
        },
        danger: {
          DEFAULT: channel("danger"),
          soft: channel("danger-soft"),
        },
        warn: {
          DEFAULT: channel("warn"),
          soft: channel("warn-soft"),
        },
        "pop-bg": "var(--pop-bg)",
        "seg-thumb": "var(--seg-thumb)",
        blue: {
          400: "#409CFF",
          500: "#0A84FF",
          600: "#0060DF",
        },
        gray: {
          50: "#fafafa",
          100: "#f4f4f5",
          200: "#e4e4e7",
          300: "#d4d4d8",
          400: "#a1a1aa",
          500: "#71717a",
          600: "#636366",
          700: "#48484A",
          800: "#3A3A3C",
          900: "#2C2C2E",
          950: "#1C1C1E",
        },
        green: {
          100: "#d1fae5",
          500: "#10b981",
        },
        red: {
          100: "#fee2e2",
          500: "#ef4444",
        },
        amber: {
          100: "#fef3c7",
          500: "#f59e0b",
        },
      },
      boxShadow: {
        sm: "0 1px 2px 0 rgb(0 0 0 / 0.05)",
        md: "0 4px 6px -1px rgb(0 0 0 / 0.1), 0 2px 4px -2px rgb(0 0 0 / 0.1)",
        lg: "0 10px 15px -3px rgb(0 0 0 / 0.1), 0 4px 6px -4px rgb(0 0 0 / 0.1)",
        pop: "var(--shadow-pop)",
        dialog: "var(--shadow-dialog)",
      },
      borderRadius: {
        sm: "0.375rem",
        md: "0.5rem",
        lg: "0.75rem",
        xl: "0.875rem",
        list: "12px",
        card: "10px",
        ctl: "7px",
        pop: "11px",
        dialog: "14px",
      },
      fontSize: {
        ui: "13px",
        nav: "12.5px",
        ctl: "12px",
        sub: "11.5px",
        note: "11px",
        micro: "10.5px",
        title: "15px",
        kpi: "20px",
      },
      transitionTimingFunction: {
        soft: "cubic-bezier(.2, .8, .2, 1)",
      },
      fontFamily: {
        // The stacks live in src/index.css (--font-ui is Segoe UI Variable on
        // Windows, the system face elsewhere).
        sans: ["var(--font-ui)"],
        mono: ["var(--font-code)"],
      },
      animation: {
        "fade-in": "fadeIn 0.5s ease-out",
        "slide-up": "slideUp 0.5s ease-out",
        "slide-down": "slideDown 0.3s ease-out",
        "slide-in-right": "slideInRight 0.3s ease-out",
        "pulse-slow": "pulse 3s cubic-bezier(0.4, 0, 0.6, 1) infinite",
        "accordion-down": "accordion-down 0.2s ease-out",
        "accordion-up": "accordion-up 0.2s ease-out",
        // keyframes for these three are in src/index.css, beside their
        // prefers-reduced-motion opt-out
        "pop-in":
          "pop-fade .18s ease-out, pop-in .42s cubic-bezier(.16, 1, .3, 1)",
        fade: "fade .2s cubic-bezier(.2, .8, .2, 1)",
        "dialog-in": "dialog-in .24s cubic-bezier(.2, .8, .2, 1)",
      },
      keyframes: {
        fadeIn: {
          "0%": {
            opacity: "0",
          },
          "100%": {
            opacity: "1",
          },
        },
        slideUp: {
          "0%": {
            transform: "translateY(20px)",
            opacity: "0",
          },
          "100%": {
            transform: "translateY(0)",
            opacity: "1",
          },
        },
        slideDown: {
          "0%": {
            transform: "translateY(-100%)",
            opacity: "0",
          },
          "100%": {
            transform: "translateY(0)",
            opacity: "1",
          },
        },
        slideInRight: {
          "0%": {
            transform: "translateX(100%)",
            opacity: "0",
          },
          "100%": {
            transform: "translateX(0)",
            opacity: "1",
          },
        },
        "accordion-down": {
          from: {
            height: "0",
          },
          to: {
            height: "var(--radix-accordion-content-height)",
          },
        },
        "accordion-up": {
          from: {
            height: "var(--radix-accordion-content-height)",
          },
          to: {
            height: "0",
          },
        },
      },
    },
  },
  plugins: [],
};
