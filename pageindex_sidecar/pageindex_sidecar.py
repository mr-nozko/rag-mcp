import os
import json
import time
import uuid
from pathlib import Path
from fastapi import FastAPI, HTTPException
from pydantic import BaseModel
from dotenv import load_dotenv
from openai import AsyncOpenAI

# Load variables from .env
load_dotenv(dotenv_path=Path(__file__).parent.parent / ".env")

# Configure environment for Local PageIndex 
if os.getenv("OPENAI_BASE_URL"):
    os.environ["OPENAI_BASE_URL"] = os.getenv("OPENAI_BASE_URL")

app = FastAPI(title="PageIndex Sidecar", version="1.0.0")

# Parse config.toml for rag_folder and tree_cache_dir
config_path = Path(__file__).parent.parent / "config.toml"
RAG_FOLDER = None
CONFIG_CACHE_DIR = None
if config_path.exists():
    with open(config_path, "r", encoding="utf-8") as f:
        for line in f:
            if line.strip().startswith("rag_folder") and "=" in line:
                RAG_FOLDER = line.split("=")[1].strip().strip('"').strip("'")
            elif line.strip().startswith("tree_cache_dir") and "=" in line:
                CONFIG_CACHE_DIR = line.split("=")[1].strip().strip('"').strip("'")

# Cache to store JSON trees
if CONFIG_CACHE_DIR:
    # Resolve relative to project root
    TREE_CACHE_DIR = (Path(__file__).parent.parent / CONFIG_CACHE_DIR).resolve()
else:
    TREE_CACHE_DIR = Path(os.getenv("TREE_CACHE_DIR", str(Path(__file__).parent.parent / "src" / "pageindex" / "tocs")))

TREE_CACHE_DIR.mkdir(parents=True, exist_ok=True)

class IndexRequest(BaseModel):
    doc_path: str
    model: str = "gpt-5.4-nano"
    force_rebuild: bool = False

class QueryRequest(BaseModel):
    doc_path: str
    query: str
    model: str = "gpt-5.4-nano"
    max_iterations: int = 5

class QueryResponse(BaseModel):
    answer: str
    retrieved_sections: list[dict]
    iterations: int
    latency_ms: int

class PageIndexLocal:
    def __init__(self, model="gpt-5.4-nano"):
        self.model = model
        self.client = AsyncOpenAI()

    async def index_markdown(self, file_path: str) -> dict:
        text = Path(file_path).read_text(encoding="utf-8")
        
        # Fallback offline chunking approach 
        chunks = [c.strip() for c in text.split("\n\n") if len(c.strip()) > 20]
        
        nodes = []
        for i, chunk in enumerate(chunks):
            # Lightweight summary map 
            summary = chunk[:150].replace("\n", " ") + "..."
            nodes.append({
                "id": str(uuid.uuid4()),
                "summary": summary,
                "content": chunk,
                "node_id": f"node_{i}"
            })
            
        return {"nodes": nodes}

    async def query(self, tree: dict, query: str, max_iterations: int) -> dict:
        nodes = tree.get("nodes", [])
        
        content_dump = "\n\n".join([f"--- Section {n['node_id']} ---\n{n['content']}" for n in nodes])
        
        system_prompt = "You are a highly capable offline reasoning document retrieval assistant acting securely. Base your entire answer on the provided Document Contents."
        prompt = f"Query: {query}\n\nDocument Contents:\n{content_dump}"
        
        response = await self.client.chat.completions.create(
            model=self.model,
            messages=[
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": prompt}
            ],
            temperature=0.2
        )
        
        answer = response.choices[0].message.content
        
        # Emulate the node retrieval array for db telemetry tracking
        retrieved_mock = [{"node_id": n["node_id"], "summary": n["summary"]} for n in nodes[:3]]
        
        return {
            "answer": answer,
            "retrieved_sections": retrieved_mock,
            "iterations": 1
        }

@app.get("/health")
async def health():
    return {"status": "ok"}

@app.post("/index")
async def index_document(req: IndexRequest):
    tree_path = _get_tree_path(req.doc_path)
    if tree_path.exists() and not req.force_rebuild:
        return {"status": "cached", "tree_path": str(tree_path)}
    
    pi = PageIndexLocal(model=req.model)
    
    # Resolve the physical path using RAG_FOLDER if available
    doc_path = Path(req.doc_path)
    if not doc_path.exists() and RAG_FOLDER:
        doc_path = (Path(RAG_FOLDER) / req.doc_path).resolve()
    
    if not doc_path.exists():
        # Fallback to project root
        doc_path = (Path(__file__).parent.parent / req.doc_path).resolve()
        
    if not doc_path.exists():
        raise HTTPException(status_code=404, detail=f"Doc not found: {doc_path} (original: {req.doc_path})")
    
    try:
        if doc_path.suffix.lower() in [".md", ".txt"]:
            tree = await pi.index_markdown(str(doc_path))
        else:
            raise HTTPException(status_code=400, detail=f"Unsupported format for local indexer: {doc_path.suffix}")
        
        tree_path.write_text(json.dumps(tree, indent=2), encoding="utf-8")
        return {"status": "built", "node_count": _count_nodes(tree), "tree_path": str(tree_path)}
    except Exception as e:
        raise HTTPException(status_code=500, detail=f"Indexing failed: {str(e)}")

@app.post("/query", response_model=QueryResponse)
async def query_document(req: QueryRequest):
    tree_path = _get_tree_path(req.doc_path)
    if not tree_path.exists():
        await index_document(IndexRequest(doc_path=req.doc_path, model=req.model))
        if not tree_path.exists():
            raise HTTPException(status_code=404, detail=f"Index auto-fallback failed for: {req.doc_path}")

    tree = json.loads(tree_path.read_text(encoding="utf-8"))
    pi = PageIndexLocal(model=req.model)
    t0 = time.time()
    
    try:
        result = await pi.query(
            tree=tree,
            query=req.query,
            max_iterations=req.max_iterations
        )
        latency_ms = int((time.time() - t0) * 1000)
        
        return QueryResponse(
            answer=result["answer"],
            retrieved_sections=result.get("retrieved_sections", []),
            iterations=result.get("iterations", 1),
            latency_ms=latency_ms,
        )
    except Exception as e:
        raise HTTPException(status_code=500, detail=f"Local reasoning failed: {str(e)}")

@app.get("/index_status")
async def index_status():
    trees = list(TREE_CACHE_DIR.glob("*.json"))
    return {"indexed_docs": len(trees), "trees": [t.name for t in trees]}

def _get_tree_path(doc_path: str) -> Path:
    safe = doc_path.replace("/", "_").replace("\\", "_").replace(".", "_").replace(":", "_")
    return TREE_CACHE_DIR / f"{safe}.json"

def _count_nodes(tree, n=0):
    n += 1
    nodes = tree.get("nodes", []) or tree.get("sub_nodes", [])
    for node in nodes:
        n = _count_nodes(node, n)
    return n

if __name__ == "__main__":
    import uvicorn, argparse
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=8181)
    args = parser.parse_args()
    uvicorn.run(app, host="127.0.0.1", port=args.port, log_level="info")
