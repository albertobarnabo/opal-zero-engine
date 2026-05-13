import type { Metadata } from "next";
import { Inter } from "next/font/google";
import "./globals.css";

const inter = Inter({
  subsets: ["latin"],
  variable: "--font-inter",
  display: "swap",
});

export const metadata: Metadata = {
  title: "Axion",
  description: "AI Agent Mission Control",
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" className={inter.variable} style={{ overflow: "hidden", height: "100%" }}>
      <body
        className={`${inter.className} antialiased`}
        style={{ overflow: "hidden", height: "100%", margin: 0 }}
      >
        {children}
      </body>
    </html>
  );
}
