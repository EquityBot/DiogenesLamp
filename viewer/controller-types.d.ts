import type { CatalogueAlbum, ViewState } from './view-state.js';
import type { Prefs } from './prefs.js';

export type ViewerAlbum = Omit<CatalogueAlbum, 'links'> & {
  title: string;
  album_url: string;
  thumb: string;
  links: {platform: string; url: string}[];
  last_seen?: string;
  thumb_version?: string;
  date_modified?: string;
  description?: string;
  description_truncated?: boolean;
  cover_hash?: string;
  price_evidence?: {token?: string; rule?: string; field?: string; candidates?: unknown[]};
  history?: {at: string; field?: string; before?: unknown; after?: unknown}[];
};
export type Seller = {id: string; name: string; album_count: number; shard: string;
  album_ids?: string[];
  run?: {last_success?: string; failed_ids?: string[]}};
export type Shop = {name: string; url: string; platform: string};
export type CatalogueIndex = {schema_version?: 1; generated_at: string; sellers: Seller[]; shops: Shop[];
  selection_version?: 1;
  album_aliases?: Record<string, string>};
export type CatalogueShard = {schema_version?: 1; albums: ViewerAlbum[]};
export type ViewerState = {
  index: CatalogueIndex | null;
  albums: ViewerAlbum[];
  albumAliases: Record<string, string>;
  filtered: ViewerAlbum[];
  favorites: Set<string>;
  collections: Map<string, string>;
  tags: Map<string, string[]>;
  reviews: Map<string, number>;
  loadedSellers: number;
  pendingShards: Seller[];
  generatedAt: string;
  dupCounts: Map<string, number>;
  dupHidden: number;
  view: ViewState;
  bannedKeys: Set<string>;
  titleKeyCounts: Map<string, number>;
  shardAlbums: Map<string, ViewerAlbum[]>;
  failedShards: string[];
  categories: Set<string>;
  filterVersion: number;
  prefs: Prefs;
};

// These elements belong to index.html and exist before the controller scripts run.
export interface ViewerElements {
  'price-min': HTMLInputElement; 'price-max': HTMLInputElement;
  'include-unknown': HTMLInputElement; 'collection-filter': HTMLSelectElement;
  'tag-filter': HTMLSelectElement; unreviewed: HTMLInputElement; changed: HTMLInputElement;
  'kind-filter': HTMLSelectElement; 'description-coverage': HTMLElement;
  search: HTMLInputElement; 'seller-filter': HTMLSelectElement; 'category-filter': HTMLSelectElement;
  'new-only': HTMLInputElement; 'favs-only': HTMLInputElement; 'hide-dups': HTMLInputElement;
  'age-filter': HTMLSelectElement; 'sort-filter': HTMLSelectElement;
  'export-favs': HTMLButtonElement; 'group-clear': HTMLButtonElement; 'filters-clear': HTMLButtonElement;
  status: HTMLElement; grid: HTMLDivElement; 'grid-spacer': HTMLDivElement;
  'grid-container': HTMLElement; 'grid-message': HTMLElement;
  'load-progress-bar': HTMLDivElement & {parentElement: HTMLDivElement};
  'load-note': HTMLDivElement; 'load-note-text': HTMLElement; 'load-note-dismiss': HTMLButtonElement;
  'shops-panel': HTMLElement; 'shops-toggle': HTMLButtonElement; 'theme-toggle': HTMLButtonElement;
  'theme-icon': SVGUseElement; 'persistence-note': HTMLElement; 'price-error': HTMLElement;
  'album-preview': HTMLDialogElement; 'action-status': HTMLElement; 'saved-search': HTMLSelectElement;
  'import-backup': HTMLInputElement; 'import-sheet': HTMLInputElement; 'export-csv': HTMLButtonElement;
  'save-search': HTMLButtonElement; 'search-name': HTMLInputElement; 'remove-search': HTMLButtonElement;
  'mark-reviewed': HTMLButtonElement; 'preview-favourite': HTMLButtonElement;
  'preview-price': HTMLElement; 'preview-description': HTMLElement; 'preview-links': HTMLDivElement;
  'preview-external': HTMLDivElement; 'preview-title': HTMLElement; 'preview-position': HTMLElement;
  'preview-prev': HTMLButtonElement; 'preview-next': HTMLButtonElement; 'preview-facts': HTMLElement;
  'preview-match': HTMLElement; 'preview-run': HTMLElement; 'preview-cover': HTMLImageElement;
  'preview-no-cover': HTMLElement; 'preview-history': HTMLUListElement;
  'preview-collection': HTMLInputElement; 'preview-tags': HTMLInputElement; 'preview-notes': HTMLTextAreaElement;
  'preview-save-status': HTMLElement; 'preview-similar': HTMLDetailsElement;
  'preview-save': HTMLButtonElement; 'preview-recovery': HTMLDivElement;
  'preview-retain': HTMLButtonElement; 'preview-transfer': HTMLButtonElement; 'preview-discard': HTMLButtonElement;
  'similar-results': HTMLUListElement; 'preview-close': HTMLButtonElement; 'preview-form': HTMLFormElement;
  'pref-agent': HTMLSelectElement; 'pref-lookups': HTMLInputElement;
  'pref-currency': HTMLInputElement; 'pref-rate': HTMLInputElement; topbar: HTMLElement;
}

declare global {
  interface Document {
    getElementById<K extends keyof ViewerElements>(id: K): ViewerElements[K];
  }
}
