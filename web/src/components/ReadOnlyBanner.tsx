interface ReadOnlyBannerProps {
  /** Link to API docs or a specific section (optional). */
  docsHref?: string;
}

/**
 * Non-dismissible banner for pages that display data but do not allow edits
 * through the UI. Renders at the top of the page to set operator expectations.
 */
export function ReadOnlyBanner({ docsHref }: ReadOnlyBannerProps) {
  return (
    <div
      role="status"
      aria-live="polite"
      className="flex items-center gap-2 rounded-md border border-yellow-300 bg-yellow-50 px-4 py-3 text-sm text-yellow-800 dark:border-yellow-700 dark:bg-yellow-950 dark:text-yellow-300"
    >
      <span aria-hidden="true">⚠️</span>
      <span>
        This page is read-only. Use the{' '}
        {docsHref ? (
          <a
            href={docsHref}
            target="_blank"
            rel="noreferrer"
            className="font-medium underline underline-offset-2 hover:no-underline"
          >
            API
          </a>
        ) : (
          'API'
        )}{' '}
        to make changes.
      </span>
    </div>
  );
}
