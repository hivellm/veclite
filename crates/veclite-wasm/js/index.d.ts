// Type definitions for @veclite/wasm (SPEC-012). The surface mirrors SPEC-010
// (the Node binding) in camelCase, minus file paths / locks / vacuum, and every
// method is async (WASM-020).

export type Metric = 'cosine' | 'euclidean' | 'dotproduct';
export type WasmVariant = 'simd128' | 'fallback';

export interface HnswOptions {
  m?: number;
  efConstruction?: number;
  efSearch?: number;
}

export interface CollectionOptions {
  dimension: number;
  metric?: Metric;
  quantizationBits?: number;
  hnsw?: HnswOptions;
  /** Auto-embed provider id (e.g. "bm25"); absent = bring-your-own vectors. */
  autoEmbed?: string;
  /** Declared payload indexes as `[key, "keyword" | "integer" | "float"]` pairs. */
  payloadIndexes?: Array<[string, string]>;
}

export interface SearchOptions {
  limit?: number;
  efSearch?: number;
  withPayload?: boolean;
  withVector?: boolean;
  /** A Qdrant-style filter document (SPEC-006). */
  filter?: unknown;
}

export interface SparseVector {
  indices: number[];
  values: number[];
}

export interface HybridOptions {
  dense?: Float32Array | number[];
  sparse?: SparseVector;
  alpha?: number;
  rrfK?: number;
  limit?: number;
}

export interface ScrollOptions {
  limit?: number;
  offsetId?: string;
  filter?: unknown;
}

export interface Hit {
  id: string;
  score: number;
  payload?: unknown;
  vector?: Float32Array;
}

export interface ScrollPage {
  points: Hit[];
  nextCursor: string | null;
}

export interface Stats {
  name: string;
  dimension: number;
  len: number;
  tombstones: number;
  autoEmbed: boolean;
}

export interface BatchPoint {
  id: string;
  vector: Float32Array | number[];
  payload?: unknown;
  sparse?: SparseVector;
}

export interface ChunkSpan {
  text: string;
  start: number;
  end: number;
}

/** Autosave policy for an OPFS-backed database (WASM-011). */
export interface AutosaveOptions {
  afterWrites?: number;
  intervalMs?: number;
}

export interface OpenOptions {
  /** OPFS file name; enables `save()` and autosave. Absent = in-memory only. */
  opfs?: string;
  autosave?: AutosaveOptions;
}

export declare class Collection {
  len(): Promise<number>;
  isEmpty(): Promise<boolean>;
  upsert(
    id: string,
    vector: Float32Array | number[],
    payload?: unknown,
    sparse?: SparseVector,
  ): Promise<void>;
  upsertBatch(points: BatchPoint[]): Promise<void>;
  upsertText(id: string, text: string, payload?: unknown): Promise<void>;
  refit(): Promise<void>;
  search(query: Float32Array | number[], options?: SearchOptions): Promise<Hit[]>;
  searchText(query: string, options?: SearchOptions): Promise<Hit[]>;
  hybridSearch(options: HybridOptions): Promise<Hit[]>;
  scroll(options?: ScrollOptions): Promise<ScrollPage>;
  get(id: string): Promise<Hit | null>;
  delete(id: string): Promise<boolean>;
  stats(): Promise<Stats>;
}

export declare class Database {
  createCollection(name: string, options: CollectionOptions): Promise<Collection>;
  collection(name: string): Promise<Collection>;
  listCollections(): Promise<string[]>;
  deleteCollection(name: string): Promise<void>;
  createAlias(alias: string, target: string): Promise<void>;
  deleteAlias(alias: string): Promise<void>;
  formatVersion(): number;
  /** No-op (compaction happens on serialize/save). */
  vacuum(): Promise<void>;
  /** The `.veclite` v1 file image (WASM-010), interchangeable with native VecLite. */
  serialize(): Promise<Uint8Array>;
  /** Checkpoint-serialize and write atomically to OPFS (WASM-011). */
  save(): Promise<void>;
  /** Flush (if OPFS-backed) and stop the autosave timer. Idempotent. */
  close(): Promise<void>;
}

/** Initialize the wasm module (idempotent); resolves to the loaded variant. */
export declare function ready(): Promise<WasmVariant>;
/** The loaded wasm variant, or `null` before `ready()` resolves. */
export declare function loadedVariant(): WasmVariant | null;
/** An ephemeral in-memory database (lost on unload). */
export declare function memory(): Promise<Database>;
/** Open a database; `{ opfs }` persists via OPFS, otherwise in-memory. */
export declare function open(options?: OpenOptions): Promise<Database>;
/** Load a database from a `.veclite` v1 file image (WASM-010). */
export declare function deserialize(bytes: Uint8Array | ArrayBuffer): Promise<Database>;
/** Split text into overlapping, UTF-8-safe chunks (SPEC-005 §7). */
export declare function chunk(
  text: string,
  maxChars?: number,
  overlap?: number,
): Promise<ChunkSpan[]>;
