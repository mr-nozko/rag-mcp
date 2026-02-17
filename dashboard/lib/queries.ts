import { getDatabase } from './db';
import type { 
  IndexStats, 
  NamespaceStats, 
  QueryLog, 
  HealthMetrics,
  EntityRelation,
  Document,
  Chunk,
  DocumentDetails 
} from './types';
import { getTimeRangeFilter, type TimePeriod } from './time-utils';

export function getIndexStats(timeRange: TimePeriod = 'all'): IndexStats {
  const db = getDatabase();
  const timeFilter = getTimeRangeFilter(timeRange);
  const whereClause = timeFilter ? `WHERE last_modified >= ${timeFilter}` : '';
  
  return db.prepare(`
    SELECT 
      (SELECT COUNT(*) FROM documents ${whereClause}) as docCount,
      (SELECT COUNT(*) FROM chunks WHERE doc_id IN (SELECT doc_id FROM documents ${whereClause})) as chunkCount,
      (SELECT COUNT(*) FROM chunks WHERE embedding IS NOT NULL AND doc_id IN (SELECT doc_id FROM documents ${whereClause})) as embeddedCount,
      (SELECT SUM(content_tokens) FROM documents ${whereClause}) as totalTokens,
      (SELECT MAX(last_modified) FROM documents ${whereClause}) as lastUpdate
  `).get() as IndexStats;
}

export function getNamespaceDistribution(timeRange: TimePeriod = 'all'): NamespaceStats[] {
  const db = getDatabase();
  const timeFilter = getTimeRangeFilter(timeRange);
  const whereClause = timeFilter ? `WHERE d.last_modified >= ${timeFilter}` : '';
  
  return db.prepare(`
    SELECT 
      d.namespace,
      COUNT(DISTINCT d.doc_id) as docCount,
      COUNT(c.chunk_id) as chunkCount,
      CAST(COUNT(CASE WHEN c.embedding IS NOT NULL THEN 1 END) * 100.0 / NULLIF(COUNT(c.chunk_id), 0) AS INTEGER) as embeddingCoverage
    FROM documents d
    LEFT JOIN chunks c ON d.doc_id = c.doc_id
    ${whereClause}
    GROUP BY d.namespace
    ORDER BY docCount DESC
  `).all() as NamespaceStats[];
}

export function getRecentQueries(limit: number = 10, timeRange: TimePeriod = 'all'): QueryLog[] {
  const db = getDatabase();
  const timeFilter = getTimeRangeFilter(timeRange);
  const whereClause = timeFilter ? `WHERE timestamp >= ${timeFilter}` : '';
  
  return db.prepare(`
    SELECT 
      query_id as queryId,
      timestamp,
      query_text as queryText,
      namespace,
      retrieval_method as retrievalMethod,
      latency_ms as latencyMs,
      result_count as resultCount
    FROM query_logs
    ${whereClause}
    ORDER BY timestamp DESC
    LIMIT ?
  `).all(limit) as QueryLog[];
}

export function getHealthMetrics(timeRange: TimePeriod = '1d'): HealthMetrics {
  const db = getDatabase();
  const timeFilter = getTimeRangeFilter(timeRange);
  const whereClause = timeFilter 
    ? `WHERE latency_ms IS NOT NULL AND timestamp >= ${timeFilter}`
    : `WHERE latency_ms IS NOT NULL`;
  
  const latencies = db.prepare(`
    SELECT latency_ms FROM query_logs 
    ${whereClause}
    ORDER BY latency_ms
  `).all() as { latency_ms: number }[];
  
  if (latencies.length === 0) return { p50: 0, p95: 0, status: 'no_data' };
  
  const p50 = latencies[Math.floor(latencies.length * 0.5)]?.latency_ms || 0;
  const p95 = latencies[Math.floor(latencies.length * 0.95)]?.latency_ms || 0;
  
  const status = p95 < 1000 ? 'excellent' : p95 < 2000 ? 'good' : 'degraded';
  
  return { p50, p95, status };
}

// ========== Knowledge Graph Queries ==========

export function getEntityRelations(limit: number = 500): EntityRelation[] {
  const db = getDatabase();
  return db.prepare(`
    SELECT 
      relation_id as relationId,
      source_entity as sourceEntity,
      relation_type as relationType,
      target_entity as targetEntity,
      metadata_json as metadataJson
    FROM entity_relations
    LIMIT ?
  `).all(limit) as EntityRelation[];
}

export function getDocumentsByNamespace(namespace: string): Document[] {
  const db = getDatabase();
  return db.prepare(`
    SELECT 
      doc_id as docId,
      doc_path as docPath,
      doc_type as docType,
      namespace,
      agent_name as agentName,
      content_text as contentText,
      content_tokens as contentTokens,
      last_modified as lastModified,
      file_hash as fileHash,
      metadata_json as metadataJson
    FROM documents
    WHERE namespace = ?
    ORDER BY last_modified DESC
  `).all(namespace) as Document[];
}

export function getAllDocuments(limit: number = 100): Document[] {
  const db = getDatabase();
  return db.prepare(`
    SELECT 
      doc_id as docId,
      doc_path as docPath,
      doc_type as docType,
      namespace,
      agent_name as agentName,
      content_text as contentText,
      content_tokens as contentTokens,
      last_modified as lastModified,
      file_hash as fileHash,
      metadata_json as metadataJson
    FROM documents
    ORDER BY last_modified DESC
    LIMIT ?
  `).all(limit) as Document[];
}

export function getDocumentDetails(docId: string): DocumentDetails | null {
  const db = getDatabase();
  
  // Get document
  const doc = db.prepare(`
    SELECT 
      doc_id as docId,
      doc_path as docPath,
      doc_type as docType,
      namespace,
      agent_name as agentName,
      content_text as contentText,
      content_tokens as contentTokens,
      last_modified as lastModified,
      file_hash as fileHash,
      metadata_json as metadataJson
    FROM documents
    WHERE doc_id = ?
  `).get(docId) as Document | undefined;
  
  if (!doc) return null;
  
  // Get chunks
  const chunks = db.prepare(`
    SELECT 
      chunk_id as chunkId,
      doc_id as docId,
      chunk_index as chunkIndex,
      chunk_text as chunkText,
      chunk_tokens as chunkTokens,
      section_header as sectionHeader,
      chunk_type as chunkType,
      embedding
    FROM chunks
    WHERE doc_id = ?
    ORDER BY chunk_index
  `).all(docId) as Chunk[];
  
  // Get related entities (where this doc is source or target)
  const entityPattern = `%${doc.docPath}%`;
  const relations = db.prepare(`
    SELECT 
      relation_id as relationId,
      source_entity as sourceEntity,
      relation_type as relationType,
      target_entity as targetEntity,
      metadata_json as metadataJson
    FROM entity_relations
    WHERE source_entity LIKE ? OR target_entity LIKE ?
  `).all(entityPattern, entityPattern) as EntityRelation[];
  
  return {
    ...doc,
    chunks,
    relations,
  };
}

export function getRelatedDocuments(docId: string): Document[] {
  const db = getDatabase();
  
  // Get the document to find its entity references
  const doc = db.prepare(`
    SELECT doc_path FROM documents WHERE doc_id = ?
  `).get(docId) as { doc_path: string } | undefined;
  
  if (!doc) return [];
  
  // Find related entities
  const entityPattern = `%${doc.doc_path}%`;
  const relatedEntities = db.prepare(`
    SELECT DISTINCT 
      CASE 
        WHEN source_entity LIKE ? THEN target_entity
        ELSE source_entity
      END as entity
    FROM entity_relations
    WHERE source_entity LIKE ? OR target_entity LIKE ?
  `).all(entityPattern, entityPattern, entityPattern) as { entity: string }[];
  
  if (relatedEntities.length === 0) return [];
  
  // Find documents matching those entities
  const entityConditions = relatedEntities.map(() => 'doc_path LIKE ?').join(' OR ');
  const entityParams = relatedEntities.map(e => `%${e.entity}%`);
  
  return db.prepare(`
    SELECT 
      doc_id as docId,
      doc_path as docPath,
      doc_type as docType,
      namespace,
      agent_name as agentName,
      content_text as contentText,
      content_tokens as contentTokens,
      last_modified as lastModified,
      file_hash as fileHash,
      metadata_json as metadataJson
    FROM documents
    WHERE doc_id != ? AND (${entityConditions})
    LIMIT 20
  `).all(docId, ...entityParams) as Document[];
}

// ========== Embedding Queries ==========

export interface EmbeddingStats {
  totalChunks: number;
  embeddedChunks: number;
  coveragePercent: number;
  byNamespace: Array<{
    namespace: string;
    totalChunks: number;
    embeddedChunks: number;
    coveragePercent: number;
  }>;
}

export function getEmbeddingStats(): EmbeddingStats {
  const db = getDatabase();
  
  // Overall stats
  const overall = db.prepare(`
    SELECT 
      COUNT(*) as totalChunks,
      COUNT(CASE WHEN embedding IS NOT NULL THEN 1 END) as embeddedChunks
    FROM chunks
  `).get() as { totalChunks: number; embeddedChunks: number };
  
  const coveragePercent = overall.totalChunks > 0 
    ? Math.round((overall.embeddedChunks / overall.totalChunks) * 100)
    : 0;
  
  // By namespace
  const byNamespace = db.prepare(`
    SELECT 
      d.namespace,
      COUNT(c.chunk_id) as totalChunks,
      COUNT(CASE WHEN c.embedding IS NOT NULL THEN 1 END) as embeddedChunks,
      CAST(COUNT(CASE WHEN c.embedding IS NOT NULL THEN 1 END) * 100.0 / NULLIF(COUNT(c.chunk_id), 0) AS INTEGER) as coveragePercent
    FROM documents d
    JOIN chunks c ON d.doc_id = c.doc_id
    GROUP BY d.namespace
    ORDER BY coveragePercent ASC, d.namespace
  `).all() as Array<{
    namespace: string;
    totalChunks: number;
    embeddedChunks: number;
    coveragePercent: number;
  }>;
  
  return {
    ...overall,
    coveragePercent,
    byNamespace,
  };
}

export interface MissingEmbedding {
  chunkId: string;
  docPath: string;
  sectionHeader: string | null;
  chunkPreview: string;
  chunkTokens: number;
}

export function getMissingEmbeddings(limit: number = 20, offset: number = 0): MissingEmbedding[] {
  const db = getDatabase();
  
  return db.prepare(`
    SELECT 
      c.chunk_id as chunkId,
      d.doc_path as docPath,
      c.section_header as sectionHeader,
      SUBSTR(c.chunk_text, 1, 100) as chunkPreview,
      c.chunk_tokens as chunkTokens
    FROM chunks c
    JOIN documents d ON c.doc_id = d.doc_id
    WHERE c.embedding IS NULL
    ORDER BY d.last_modified DESC, c.chunk_index
    LIMIT ? OFFSET ?
  `).all(limit, offset) as MissingEmbedding[];
}

export function getMissingEmbeddingsCount(): number {
  const db = getDatabase();
  
  const result = db.prepare(`
    SELECT COUNT(*) as count
    FROM chunks
    WHERE embedding IS NULL
  `).get() as { count: number };
  
  return result.count;
}

export function getChunkWithEmbedding(chunkId: string): Chunk | null {
  const db = getDatabase();
  
  const chunk = db.prepare(`
    SELECT 
      chunk_id as chunkId,
      doc_id as docId,
      chunk_index as chunkIndex,
      chunk_text as chunkText,
      chunk_tokens as chunkTokens,
      section_header as sectionHeader,
      chunk_type as chunkType,
      embedding
    FROM chunks
    WHERE chunk_id = ? AND embedding IS NOT NULL
  `).get(chunkId) as Chunk | undefined;
  
  return chunk || null;
}

export function getChunksWithEmbeddings(limit: number = 100): Chunk[] {
  const db = getDatabase();
  
  return db.prepare(`
    SELECT 
      chunk_id as chunkId,
      doc_id as docId,
      chunk_index as chunkIndex,
      chunk_text as chunkText,
      chunk_tokens as chunkTokens,
      section_header as sectionHeader,
      chunk_type as chunkType,
      embedding
    FROM chunks
    WHERE embedding IS NOT NULL
    ORDER BY RANDOM()
    LIMIT ?
  `).all(limit) as Chunk[];
}

// ========== Document Graph Queries ==========

export interface DocumentGraphData {
  nodes: Document[];
  links: Array<{ source: string; target: string; relationType: string }>;
}

export function getDocumentGraph(): DocumentGraphData {
  const db = getDatabase();
  
  // Get all documents
  const documents = db.prepare(`
    SELECT 
      doc_id as docId,
      doc_path as docPath,
      doc_type as docType,
      namespace,
      agent_name as agentName,
      content_text as contentText,
      content_tokens as contentTokens,
      last_modified as lastModified,
      file_hash as fileHash,
      metadata_json as metadataJson
    FROM documents
    ORDER BY last_modified DESC
    LIMIT 100
  `).all() as Document[];
  
  const links: Array<{ source: string; target: string; relationType: string }> = [];
  
  // Strategy 1: Connect documents in the same namespace
  // Group documents by namespace
  const namespaceGroups = new Map<string, Document[]>();
  documents.forEach(doc => {
    if (!namespaceGroups.has(doc.namespace)) {
      namespaceGroups.set(doc.namespace, []);
    }
    namespaceGroups.get(doc.namespace)!.push(doc);
  });
  
  // Create sparse connections within each namespace to form blob-like clusters
  // Minimal connections keep nodes together but allow them to spread into circular shapes
  namespaceGroups.forEach((docs, namespace) => {
    if (docs.length === 1) return;
    
    // Connect each node to only 1-2 random others in the same namespace
    // This creates loose clusters that naturally form circular blobs
    for (let i = 0; i < docs.length; i++) {
      // Connect to 1-2 other nodes in the cluster
      const connectionCount = docs.length <= 3 ? 1 : 2;
      
      for (let c = 0; c < connectionCount; c++) {
        // Pick a random other node (not self, not too close in index)
        const offset = Math.floor(docs.length / (connectionCount + 1)) * (c + 1);
        const targetIdx = (i + offset) % docs.length;
        
        if (i !== targetIdx) {
          links.push({
            source: docs[i].docId,
            target: docs[targetIdx].docId,
            relationType: namespace
          });
        }
      }
    }
  });
  
  // Strategy 2: Cross-namespace connections via shared agent_name
  const agentGroups = new Map<string, Document[]>();
  documents.forEach(doc => {
    if (doc.agentName) {
      if (!agentGroups.has(doc.agentName)) {
        agentGroups.set(doc.agentName, []);
      }
      agentGroups.get(doc.agentName)!.push(doc);
    }
  });
  
  // Connect first two documents with same agent across different namespaces
  agentGroups.forEach((docs, agentName) => {
    if (docs.length > 1 && docs[0].namespace !== docs[1].namespace) {
      links.push({
        source: docs[0].docId,
        target: docs[1].docId,
        relationType: 'same_agent'
      });
    }
  });
  
  return { nodes: documents, links };
}
