# Chapter 1: Your First Endpoint — Health Check

Welcome to the Movie Watchlist API course! In this first chapter, you'll create a FastAPI app with a single health check endpoint.

## What you'll build

A `GET /health` endpoint that returns:

```json
{"status": "ok"}
```

## Create your project

Create a file called `main.py` in your project folder:

```python
from fastapi import FastAPI

app = FastAPI()

@app.get("/health")
def health_check():
    return {"status": "ok"}
```

## Run your server

```bash
uvicorn main:app --reload --port 8000
```

Visit `http://localhost:8000/health` — you should see `{"status":"ok"}`.

When you're ready, click **Run Tests** to verify your implementation.
