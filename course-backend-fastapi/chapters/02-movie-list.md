# Chapter 2: Returning Data — The Movie List

Now that your server is running, let's add a `GET /movies` endpoint that returns a list of movies.

## What you'll build

A `GET /movies` endpoint that returns an array of movie objects. Each movie must have:

- `id` (number)
- `title` (string)
- `year` (number)
- `genre` (string)
- `watched` (boolean)

## Implementation

Add an in-memory list and endpoint to `main.py`:

```python
from fastapi import FastAPI

app = FastAPI()

movies = [
    {"id": 1, "title": "The Matrix", "year": 1999, "genre": "Sci-Fi", "watched": False},
]

@app.get("/health")
def health_check():
    return {"status": "ok"}

@app.get("/movies")
def get_movies():
    return movies
```

When you're ready, click **Run Tests**.
