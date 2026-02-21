# Chapter 3: Creating Resources — Adding a Movie

Time to accept data from the client. You'll add a `POST /movies` endpoint that creates a new movie.

## What you'll build

A `POST /movies` endpoint that:
- Accepts a JSON body with `title`, `year`, `genre`
- Returns the created movie with `id`, `watched: false`, and status `201`
- Rejects missing required fields with `422`

## Implementation

```python
from fastapi import FastAPI
from pydantic import BaseModel

app = FastAPI()

movies = []
next_id = 1

class MovieIn(BaseModel):
    title: str
    year: int
    genre: str

@app.get("/health")
def health_check():
    return {"status": "ok"}

@app.get("/movies")
def get_movies():
    return movies

@app.post("/movies", status_code=201)
def add_movie(movie: MovieIn):
    global next_id
    new_movie = {"id": next_id, **movie.dict(), "watched": False}
    movies.append(new_movie)
    next_id += 1
    return new_movie
```

When you're ready, click **Run Tests**.
