import type { Metadata } from "next";
import { Space_Grotesk, Inter_Tight, JetBrains_Mono } from "next/font/google";
import "./globals.css";

const spaceGrotesk = Space_Grotesk({
  subsets: ["latin"],
  weight: ["300", "400", "500", "600", "700"],
  variable: "--font-display",
  display: "swap",
});

const interTight = Inter_Tight({
  subsets: ["latin"],
  weight: ["300", "400", "500", "600", "700"],
  variable: "--font-sans",
  display: "swap",
});

const jetbrainsMono = JetBrains_Mono({
  subsets: ["latin"],
  weight: ["400", "500"],
  variable: "--font-mono",
  display: "swap",
});

export const metadata: Metadata = {
  title: "Axion",
  description: "AI Agent Mission Control",
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html
      lang="en"
      className={`${spaceGrotesk.variable} ${interTight.variable} ${jetbrainsMono.variable}`}
      style={{ overflow: "hidden", height: "100%" }}
    >
      <body style={{ overflow: "hidden", height: "100%", margin: 0 }}>
        {/*
          Fixed background layer — guarantees the gradient mesh fills the entire
          viewport regardless of overflow:hidden on html/body. CSS background-attachment:fixed
          on overflow:hidden roots is unreliable in Chrome; a position:fixed element is not.
        */}
        <div
          aria-hidden="true"
          style={{
            position: "fixed",
            inset: 0,
            zIndex: -2,
            backgroundColor: "var(--opalzero-bg)",
            backgroundImage: "var(--opalzero-gradient-mesh)",
            pointerEvents: "none",
          }}
        />
        {/* Grid-faint overlay — same as landing page Hero */}
        <div
          aria-hidden="true"
          className="grid-faint"
          style={{
            position: "fixed",
            inset: 0,
            zIndex: -1,
            pointerEvents: "none",
          }}
        />
        {children}
      </body>
    </html>
  );
}
