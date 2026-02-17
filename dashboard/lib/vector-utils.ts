/**
 * Vector utilities for embedding analysis
 */

/**
 * Decode embedding BLOB to float array
 * SQLite stores embeddings as BLOB (binary), needs to be converted to float32 array
 */
export function decodeEmbedding(blob: Buffer | null): number[] | null {
  if (!blob) return null;
  
  const floatArray = new Float32Array(blob.buffer, blob.byteOffset, blob.length / 4);
  return Array.from(floatArray);
}

/**
 * Calculate cosine similarity between two vectors
 * Returns value between -1 and 1, where 1 is most similar
 */
export function cosineSimilarity(vecA: number[], vecB: number[]): number {
  if (vecA.length !== vecB.length) {
    throw new Error('Vectors must have same length');
  }
  
  let dotProduct = 0;
  let normA = 0;
  let normB = 0;
  
  for (let i = 0; i < vecA.length; i++) {
    dotProduct += vecA[i] * vecB[i];
    normA += vecA[i] * vecA[i];
    normB += vecB[i] * vecB[i];
  }
  
  normA = Math.sqrt(normA);
  normB = Math.sqrt(normB);
  
  if (normA === 0 || normB === 0) {
    return 0;
  }
  
  return dotProduct / (normA * normB);
}

/**
 * Get similarity category based on score
 */
export function getSimilarityCategory(score: number): 'high' | 'medium' | 'low' {
  if (score >= 0.8) return 'high';
  if (score >= 0.6) return 'medium';
  return 'low';
}

/**
 * Get color for similarity score
 */
export function getSimilarityColor(score: number): string {
  if (score >= 0.8) return 'emerald';
  if (score >= 0.6) return 'amber';
  return 'red';
}
