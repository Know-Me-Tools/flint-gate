import { createContext, useCallback, useContext, useState } from 'react';
import { cn } from '@/lib/utils';

interface ToastItem {
  id: number;
  title: string;
  description?: string;
  variant?: 'default' | 'success' | 'error';
}

interface ToastContextValue {
  toast: (t: Omit<ToastItem, 'id'>) => void;
}

const ToastCtx = createContext<ToastContextValue>({ toast: () => {} });

export function ToastProvider({ children }: { children: React.ReactNode }) {
  const [toasts, setToasts] = useState<ToastItem[]>([]);

  const toast = useCallback((t: Omit<ToastItem, 'id'>) => {
    const id = Date.now();
    setToasts((prev) => [...prev, { ...t, id }]);
    setTimeout(() => setToasts((prev) => prev.filter((x) => x.id !== id)), 4000);
  }, []);

  return (
    <ToastCtx.Provider value={{ toast }}>
      {children}
      <div className="fixed bottom-4 right-4 flex flex-col gap-2 z-50">
        {toasts.map((t) => (
          <div
            key={t.id}
            className={cn(
              'rounded-md border px-4 py-3 shadow-md text-sm',
              t.variant === 'error'
                ? 'bg-destructive text-white border-destructive'
                : t.variant === 'success'
                  ? 'bg-green-100 text-green-900 border-green-200'
                  : 'bg-background text-foreground',
            )}
          >
            <p className="font-medium">{t.title}</p>
            {t.description && <p className="text-xs mt-1 opacity-80">{t.description}</p>}
          </div>
        ))}
      </div>
    </ToastCtx.Provider>
  );
}

export function useToast() {
  return useContext(ToastCtx);
}
