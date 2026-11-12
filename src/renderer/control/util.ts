import { useCallback, useEffect, useRef, useState } from 'react';
import type { RcApi, SourceState } from '../../shared/types';

/** Access window.rc; throws a readable error when the preload bridge is missing. */
export function rc(): RcApi {
  const api = (window as Partial<Window>).rc;
  if (!api) throw new Error('Brak połączenia z aplikacją (window.rc).');
  return api;
}

export function errText(e: unknown): string {
  if (e instanceof Error) {
    // Electron prefixes IPC errors with "Error invoking remote method 'x': Error: ..."
    const m = /Error invoking remote method '[^']+': (?:\w*Error: )?(.*)$/s.exec(e.message);
    return (m?.[1] ?? e.message) || 'Nieznany błąd.';
  }
  return typeof e === 'string' ? e : 'Nieznany błąd.';
}

/** Run async rc calls with busy + inline error state. Never throws. */
export function useAction() {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const alive = useRef(true);
  useEffect(() => () => { alive.current = false; }, []);

  const run = useCallback(async <T,>(fn: () => Promise<T>): Promise<{ ok: true; value: T } | { ok: false }> => {
    setBusy(true);
    setError(null);
    try {
      const value = await fn();
      return { ok: true, value };
    } catch (e) {
      if (alive.current) setError(errText(e));
      return { ok: false };
    } finally {
      if (alive.current) setBusy(false);
    }
  }, []);

  return { run, busy, error, setError };
}

export function fmtClock(totalSec: number): string {
  const s = Math.max(0, Math.floor(totalSec));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const ss = String(s % 60).padStart(2, '0');
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${ss}` : `${String(m).padStart(2, '0')}:${ss}`;
}

export const fmtMs = (ms: number) => fmtClock(ms / 1000);

export function fmtMB(bytes: number): string {
  return `${(bytes / 1048576).toFixed(1)} MB`;
}

export function fmtDate(ts: number): string {
  return new Date(ts).toLocaleString('pl-PL', {
    day: '2-digit', month: '2-digit', year: 'numeric', hour: '2-digit', minute: '2-digit',
  });
}

export const STATE_LABEL: Record<SourceState, string> = {
  off: 'wyłączone',
  waiting: 'czeka',
  connected: 'połączono',
  error: 'błąd',
};

const LCU_PHASE: Record<string, string> = {
  None: 'bezczynny',
  Lobby: 'lobby',
  Matchmaking: 'szukanie gry',
  CheckedIntoTournament: 'turniej',
  ReadyCheck: 'akceptacja',
  ChampSelect: 'wybór postaci',
  GameStart: 'start gry',
  InProgress: 'w grze',
  Reconnect: 'ponowne łączenie',
  WaitingForStats: 'czeka na wynik',
  PreEndOfGame: 'koniec gry',
  EndOfGame: 'koniec gry',
  FailedToLaunch: 'błąd uruchomienia',
  TerminatedInError: 'zakończono błędem',
};

export function lcuPhaseLabel(phase: string | null): string | null {
  if (!phase) return null;
  return LCU_PHASE[phase] ?? phase;
}

export const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));
