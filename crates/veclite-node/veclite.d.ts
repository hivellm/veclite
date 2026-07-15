// Public TypeScript surface (SPEC-010 NODE-003). Re-exports the napi-generated
// classes/functions from `index.d.ts` and adds the `VecLiteError` class the JS
// wrapper attaches (NODE-020).

export * from './index';

/** Error thrown (sync) or rejected (async) by every VecLite operation. */
export class VecLiteError extends Error {
  /** Stable machine-readable code, e.g. `"DIMENSION_MISMATCH"`. */
  code: string;
}
