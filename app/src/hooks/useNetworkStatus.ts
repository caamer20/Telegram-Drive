import { useState, useEffect, useRef } from 'react';

/**
 * Polls Telegram connection status every 10 seconds.
 * Primary: cmd_check_connection (get_me ping via grammers client).
 * Fallback: cmd_is_network_available (TCP probe, used during auth flow).
 */
export function useNetworkStatus() {
    const [isOnline, setIsOnline] = useState(true);
    const isCheckingRef = useRef(false); // prevents overlapping poll cycles

    useEffect(() => {
        let interval: ReturnType<typeof setInterval>;

        import('@tauri-apps/api/core').then(({ invoke }) => {
            const checkNetwork = async () => {
                if (isCheckingRef.current) return;
                isCheckingRef.current = true;

                try {
                    const connected = await invoke<boolean>('cmd_check_connection');
                    setIsOnline(connected);
                } catch {
                    // fallback: no client yet (auth flow)
                    try {
                        const available = await invoke<boolean>('cmd_is_network_available');
                        setIsOnline(available);
                    } catch {
                        setIsOnline(false);
                    }
                } finally {
                    isCheckingRef.current = false;
                }
            };

            checkNetwork();
            interval = setInterval(checkNetwork, 10000);
        });

        return () => clearInterval(interval);
    }, []);

    return isOnline;
}
