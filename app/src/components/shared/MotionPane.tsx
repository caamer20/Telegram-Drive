import { type MouseEventHandler, type ReactNode } from 'react';
import { motion, useIsPresent } from 'framer-motion';
import { useMotionPolicy } from '../../context/MotionContext';

/** A direct, keyed AnimatePresence child in a positioned container.
 * Current content mounts immediately; the outgoing visual cannot receive input.
 * Do not use exit retention around account, vault or permission boundaries.
 */
export function MotionPane({ children, className, label, overlay = false, onClick }: { children: ReactNode; className?: string; label?: string; overlay?: boolean; onClick?: MouseEventHandler<HTMLDivElement> }) {
  // Observe presence without registering another exit-completion participant.
  const present = useIsPresent();
  const { reducedMotion, transition } = useMotionPolicy();
  return (
    <motion.div
      data-ui-motion
      onClick={onClick}
      aria-label={label}
      aria-hidden={!present || undefined}
      inert={!present}
      hidden={!present && reducedMotion}
      initial={reducedMotion ? false : { opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={transition(120)}
      className={className}
      style={present ? undefined : overlay ? { pointerEvents: 'none' } : { position: 'absolute', inset: 0, pointerEvents: 'none' }}
    >
      {children}
    </motion.div>
  );
}
