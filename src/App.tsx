import { motion, AnimatePresence } from 'framer-motion';
import { useState, useEffect } from 'react';
import './App.css';

/**
 * Main application container for Thuki.
 *
 * Provides a frameless, transparent interface using glassmorphism aesthetics.
 * Features entry animations and a search-oriented interface for assistant interactions.
 *
 * @returns {JSX.Element} The rendered application component hierarchy.
 */
function App() {
  const [isInitialized, setIsInitialized] = useState(false);
  const [query, setQuery] = useState('');

  useEffect(() => {
    const timer = setTimeout(() => setIsInitialized(true), 1200);
    return () => clearTimeout(timer);
  }, []);

  return (
    <AnimatePresence>
      {!isInitialized ? (
        <motion.div
          key="loader"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          className="flex items-center justify-center h-screen bg-transparent"
        >
          <motion.div
            animate={{ rotate: 360 }}
            transition={{ duration: 1, repeat: Infinity, ease: 'linear' }}
            className="w-12 h-12 rounded-full border-4 border-glass-secondary border-t-brand/80"
          />
        </motion.div>
      ) : (
        <motion.main
          key="app"
          initial={{ opacity: 0, scale: 0.95, y: 10 }}
          animate={{ opacity: 1, scale: 1, y: 0 }}
          className="flex flex-col h-screen w-screen p-10 bg-glass-base backdrop-blur-2xl border border-glass-border rounded-3xl shadow-glass"
          data-tauri-drag-region
        >
          <header
            className="mb-10 text-center flex flex-col items-center"
            data-tauri-drag-region
          >
            <motion.img
              initial={{ filter: 'blur(10px)', opacity: 0 }}
              animate={{ filter: 'blur(0px)', opacity: 1 }}
              transition={{ delay: 0.2 }}
              src="/thuki-logo.png"
              className="w-[100px] h-[100px] mb-6 rounded-3xl shadow-[0_0_40px_rgba(88,166,255,0.4)]"
              alt="Thuki logo"
              data-tauri-drag-region
            />
            <motion.h1
              initial={{ opacity: 0, y: -10 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ delay: 0.4 }}
              data-tauri-drag-region
              className="text-5xl font-bold bg-linear-to-br from-white to-brand text-transparent bg-clip-text tracking-tight m-0"
            >
              Thuki
            </motion.h1>
            <motion.p
              className="text-sm opacity-70 mt-3 font-light tracking-wide"
              initial={{ opacity: 0 }}
              animate={{ opacity: 0.7 }}
              transition={{ delay: 0.6 }}
              data-tauri-drag-region
            >
              Ready to assist! Let&apos;s go!
            </motion.p>
          </header>

          <section className="flex-1 flex flex-col items-center justify-center">
            <motion.div
              className="w-full max-w-xl mb-8"
              initial={{ width: '20%', opacity: 0 }}
              animate={{ width: '100%', opacity: 1 }}
              transition={{ delay: 0.8, type: 'spring', stiffness: 100 }}
            >
              <input
                type="text"
                placeholder="How can I help you today?"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                autoFocus
                className="w-full px-7 py-5 rounded-2xl border border-glass-border bg-glass-secondary text-text-main text-lg outline-none shadow-lg transition-all duration-300 transform focus:border-brand focus:bg-[rgba(22,27,34,0.8)] focus:ring-4 focus:ring-brand/25 focus:-translate-y-0.5"
              />
            </motion.div>

            <motion.div
              className="flex items-center text-sm opacity-60"
              initial={{ opacity: 0 }}
              animate={{ opacity: 0.6 }}
              transition={{ delay: 1.2 }}
            >
              <span className="w-2.5 h-2.5 bg-brand rounded-full mr-3.5 shadow-[0_0_15px_#58a6ff] animate-custom-pulse"></span>
              Awaiting your request
            </motion.div>
          </section>

          <footer
            className="text-right text-xs opacity-40 font-light"
            data-tauri-drag-region
          >
            <span className="version">Thuki v0.1.0</span>
          </footer>
        </motion.main>
      )}
    </AnimatePresence>
  );
}

export default App;
