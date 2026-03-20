import { motion, AnimatePresence } from 'framer-motion';
import { useState, useEffect, useRef } from 'react';
import { useOllama } from './hooks/useOllama';
import { MarkdownRenderer } from './components/MarkdownRenderer';
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
  const { messages, streamingContent, ask, isGenerating, error } = useOllama();
  const messagesEndRef = useRef<HTMLDivElement>(null);

  const handleSubmit = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      ask(query);
      setQuery('');
    }
  };

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [messages, streamingContent]);

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
              className="text-5xl font-bold bg-[linear-gradient(to_bottom_right,white,#58a6ff)] text-transparent bg-clip-text tracking-tight m-0"
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

          <section className="flex-1 flex flex-col items-center justify-center overflow-hidden">
            {messages.length === 0 ? (
              <motion.div
                className="flex items-center text-sm opacity-60 mt-4 mb-auto"
                initial={{ opacity: 0 }}
                animate={{ opacity: 0.6 }}
                transition={{ delay: 1.2 }}
              >
                <span className="w-2.5 h-2.5 bg-brand rounded-full mr-3.5 shadow-[0_0_15px_#58a6ff] animate-custom-pulse" />
                Awaiting your request
              </motion.div>
            ) : (
              <motion.div
                className="w-full max-w-xl flex-1 overflow-y-auto mb-4 p-4 rounded-2xl bg-glass-secondary border border-glass-border custom-scrollbar text-left text-sm"
                initial={{ opacity: 0, scale: 0.95 }}
                animate={{ opacity: 1, scale: 1 }}
              >
                {messages.map((msg, i) => (
                  <div
                    key={i}
                    className={`mb-4 w-full flex ${msg.role === 'user' ? 'justify-end' : 'justify-start'}`}
                  >
                    <div
                      className={`max-w-[85%] p-3 rounded-2xl ${
                        msg.role === 'user'
                          ? 'bg-brand/20 border border-brand/30 text-white rounded-br-sm'
                          : 'bg-[rgba(22,27,34,0.6)] border border-glass-border text-gray-200 rounded-bl-sm'
                      }`}
                    >
                      {msg.role === 'user' ? (
                        <span className="whitespace-pre-wrap">
                          {msg.content}
                        </span>
                      ) : (
                        <MarkdownRenderer
                          content={msg.content}
                          className="prose-sm leading-relaxed"
                        />
                      )}
                    </div>
                  </div>
                ))}
                {isGenerating && streamingContent && (
                  <div className="mb-4 w-full flex justify-start">
                    <div className="max-w-[85%] p-3 rounded-2xl bg-[rgba(22,27,34,0.6)] border border-glass-border text-gray-200 rounded-bl-sm">
                      <MarkdownRenderer
                        content={streamingContent}
                        className="prose-sm leading-relaxed"
                      />
                    </div>
                  </div>
                )}
                {isGenerating && (
                  <div className="flex items-center text-xs opacity-50 mt-2">
                    <span className="w-1.5 h-1.5 bg-brand rounded-full mr-2 animate-bounce" />
                    Thuki is thinking...
                  </div>
                )}
                {error && (
                  <div className="text-red-400 text-xs mt-2 p-2 border border-red-900 rounded bg-red-950/30">
                    Connection Error: {error}
                  </div>
                )}
                <div ref={messagesEndRef} />
              </motion.div>
            )}

            <motion.div
              className="w-full max-w-xl mt-auto shrink-0"
              initial={{ width: '20%', opacity: 0 }}
              animate={{ width: '100%', opacity: 1 }}
              transition={{ delay: 0.8, type: 'spring', stiffness: 100 }}
            >
              <input
                type="text"
                placeholder="How can I help you today?"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                onKeyDown={handleSubmit}
                disabled={isGenerating}
                autoFocus
                className="w-full px-7 py-5 mb-2 rounded-2xl border border-glass-border bg-glass-secondary text-text-main text-lg outline-none shadow-lg transition-all duration-300 transform focus:border-brand focus:bg-[rgba(22,27,34,0.8)] focus:ring-4 focus:ring-brand/25 focus:-translate-y-0.5 disabled:opacity-50"
              />
            </motion.div>
          </section>

          <footer
            className="text-right text-xs opacity-40 font-light mt-2"
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
