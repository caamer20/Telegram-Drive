import { createContext, useContext, useEffect, useLayoutEffect, useMemo, useState, type ReactNode } from 'react';
import { MotionConfig, type Transition } from 'framer-motion';
import { useSettings } from './SettingsContext';

const ease = [0.2, 0.8, 0.2, 1] as const;
interface MotionPolicy {
  reducedMotion: boolean;
  performanceMode: boolean;
  transition: (milliseconds?: number) => Transition;
}
const MotionContext = createContext<MotionPolicy>({
  reducedMotion: true,
  performanceMode: false,
  transition: () => ({ duration: 0 }),
});

/** Accessibility is live policy; it must never rewrite a saved performance preference. */
export function MotionPolicyProvider({ children }: { children: ReactNode }) {
  const { settings, isLoaded } = useSettings();
  const [systemReduced, setSystemReduced] = useState(() => window.matchMedia('(prefers-reduced-motion: reduce)').matches);
  const reducedMotion = !isLoaded || systemReduced || settings.performanceMode;

  useEffect(() => {
    const query = window.matchMedia('(prefers-reduced-motion: reduce)');
    const changed = () => setSystemReduced(query.matches);
    changed();
    query.addEventListener('change', changed);
    return () => query.removeEventListener('change', changed);
  }, []);

  useLayoutEffect(() => {
    document.documentElement.classList.toggle('motion-reduced', reducedMotion);
    document.documentElement.classList.toggle('motion-pending', !isLoaded);
    document.body.classList.toggle('performance-mode', isLoaded && settings.performanceMode);
    if (reducedMotion) {
      // Finish only decorative UI motion. Drag transforms and media timelines
      // are direct manipulation, not animations owned by this policy.
      for (const element of document.querySelectorAll<HTMLElement>('[data-ui-motion]')) {
        for (const animation of element.getAnimations?.() ?? []) {
          try { animation.finish(); } catch { animation.cancel(); }
        }
      }
    }
  }, [isLoaded, reducedMotion, settings.performanceMode]);

  useLayoutEffect(() => () => {
    document.documentElement.classList.remove('motion-reduced', 'motion-pending');
    document.body.classList.remove('performance-mode');
  }, []);

  const policy = useMemo<MotionPolicy>(() => ({
    reducedMotion,
    performanceMode: settings.performanceMode,
    transition: (milliseconds = 160) => ({ duration: reducedMotion ? 0 : milliseconds / 1000, ease }),
  }), [reducedMotion, settings.performanceMode]);

  return (
    <MotionContext.Provider value={policy}>
      <MotionConfig reducedMotion={reducedMotion ? 'always' : 'never'} transition={policy.transition()}>
        {children}
      </MotionConfig>
    </MotionContext.Provider>
  );
}

export const useMotionPolicy = () => useContext(MotionContext);
