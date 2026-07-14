/**
 * Type-level smoke test for ReadOnlyBanner.
 *
 * This project does not have a unit test runner (vitest/jest). These
 * assertions are verified at compile time by `tsc --noEmit` / `pnpm typecheck`.
 * Add a runtime test runner and convert these to describe/it blocks when
 * @testing-library/react is installed.
 */
import type { JSX } from 'react';
import { ReadOnlyBanner } from './ReadOnlyBanner';

// Verify the component renders without props (docsHref is optional).
void ((<ReadOnlyBanner />) as JSX.Element);

// Verify the component accepts a docsHref string.
void ((<ReadOnlyBanner docsHref="/docs/api" />) as JSX.Element);

// Verify invalid prop shapes are rejected by TypeScript.
// @ts-expect-error — unknown prop must be rejected
const _withBadProp: JSX.Element = <ReadOnlyBanner unknownProp="bad" />;

export {};
