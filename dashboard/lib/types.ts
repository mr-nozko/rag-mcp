export interface IndexStats {
  docCount: number;
  chunkCount: number;
  embeddedCount: number;
  totalTokens: number;
  lastUpdate: string | null;
}

export interface NamespaceStats {
  namespace: string;
  docCount: number;
  chunkCount: number;
  embeddingCoverage: number;
}

export interface QueryLog {
  queryId: string;
  timestamp: string;
  queryText: string;
  namespace: string | null;
  retrievalMethod: string | null;
  latencyMs: number | null;
  resultCount: number | null;
}

export interface HealthMetrics {
  p50: number;
  p95: number;
  status: 'excellent' | 'good' | 'degraded' | 'no_data';
}

export interface EntityRelation {
  relationId: string;
  sourceEntity: string;
  relationType: string;
  targetEntity: string;
  metadataJson: string | null;
}

export interface Document {
  docId: string;
  docPath: string;
  docType: string;
  namespace: string;
  agentName: string | null;
  contentText: string;
  contentTokens: number;
  lastModified: string;
  fileHash: string;
  metadataJson: string | null;
}

export interface Chunk {
  chunkId: string;
  docId: string;
  chunkIndex: number;
  chunkText: string;
  chunkTokens: number;
  sectionHeader: string | null;
  chunkType: string | null;
  embedding: Buffer | null;
}

export interface DocumentDetails extends Document {
  chunks: Chunk[];
  relations: EntityRelation[];
}
