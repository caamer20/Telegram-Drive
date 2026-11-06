import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface ProxyStatus {
  reachable: boolean;
  latency_ms: number;
}

export function stopControlShortcuts(event: KeyboardEvent) {
  if ((event.target as HTMLElement).closest('button, select, summary') && !event.metaKey && !event.ctrlKey && !event.altKey) event.stopPropagation();
}

export function useTopBarController(proxyEnabled: boolean, proxyLiveStateEnabled: boolean) {
  const [proxyStatus, setProxyStatus] = useState<ProxyStatus | null>(null);
  const [showMore, setShowMore] = useState(false);
  const [showViewOptions, setShowViewOptions] = useState(false);
  const [showSearchFilters, setShowSearchFilters] = useState(false);
  const moreRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<HTMLDivElement>(null);
  const filterRef = useRef<HTMLDivElement>(null);
  const filterButtonRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (showSearchFilters) filterRef.current?.querySelector('select')?.focus();
  }, [showSearchFilters]);

  const closeSearchFilters = useCallback(() => {
    setShowSearchFilters(false);
    filterButtonRef.current?.focus();
  }, []);

  const onFilterKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Escape' && showSearchFilters) {
      event.preventDefault();
      event.stopPropagation();
      closeSearchFilters();
    }
  };

  useEffect(() => {
    if (!proxyEnabled || !proxyLiveStateEnabled) {
      setProxyStatus(null);
      return;
    }
    const checkProxy = async () => {
      try {
        setProxyStatus(await invoke<ProxyStatus>('cmd_get_proxy_status'));
      } catch {
        setProxyStatus({ reachable: false, latency_ms: -1 });
      }
    };
    void checkProxy();
    const interval = setInterval(checkProxy, 5000);
    return () => clearInterval(interval);
  }, [proxyEnabled, proxyLiveStateEnabled]);

  useEffect(() => {
    if (!showMore && !showViewOptions && !showSearchFilters) return;
    const closeOutside = (event: Event) => {
      const target = event.target as Node;
      if (!moreRef.current?.contains(target)) setShowMore(false);
      if (!viewRef.current?.contains(target)) setShowViewOptions(false);
      if (!filterRef.current?.contains(target) && !(target instanceof Element && target.closest('[data-file-filter-trigger]'))) setShowSearchFilters(false);
    };
    window.addEventListener('pointerdown', closeOutside);
    window.addEventListener('focusin', closeOutside);
    return () => {
      window.removeEventListener('pointerdown', closeOutside);
      window.removeEventListener('focusin', closeOutside);
    };
  }, [showMore, showViewOptions, showSearchFilters]);

  const toggleSearchFilters = useCallback(() => {
    setShowSearchFilters(value => !value);
    setShowMore(false);
    setShowViewOptions(false);
  }, []);

  const toggleViewOptions = useCallback(() => {
    setShowViewOptions(value => !value);
    setShowMore(false);
    setShowSearchFilters(false);
  }, []);

  const toggleMore = useCallback(() => {
    setShowMore(value => !value);
    setShowViewOptions(false);
    setShowSearchFilters(false);
  }, []);

  const runMoreAction = useCallback((action: () => void) => {
    setShowMore(false);
    action();
  }, []);

  return {
    proxyStatus,
    showMore,
    showViewOptions,
    showSearchFilters,
    moreRef,
    viewRef,
    filterRef,
    filterButtonRef,
    closeSearchFilters,
    onFilterKeyDown,
    toggleSearchFilters,
    toggleViewOptions,
    toggleMore,
    runMoreAction,
  };
}
