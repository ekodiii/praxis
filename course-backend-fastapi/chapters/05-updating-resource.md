# Chapter 5: Updating Resources — PATCH a Movie

You'll add a `PATCH /movies/{id}` endpoint that partially updates a movie.

## What you'll build

- `PATCH /movies/{id}` updates only the provided fields
- Returns the updated movie (200)
- Returns 404 if the movie doesn't exist
- Fields not provided in the body remain unchanged

## Implementation

```python
from fastapi import FastAPI, HTTPException
from pydantic import BaseModel
from typing import Optional

app = FastAPI()
movies = []
next_id = 1

class MovieIn(BaseModel):
    title: str
    year: int
    genre: str

class MoviePatch(BaseModel):
    title: Optional[str] = None
    year: Optional[int] = None
    genre: Optional[str] = None
    watched: Optional[bool] = None

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

@app.get("/movies/{movie_id}")
def get_movie(movie_id: int):
    for movie in movies:
        if movie["id"] == movie_id:
            return movie
    raise HTTPException(status_code=404, detail="Movie not found")

@app.patch("/movies/{movie_id}")
def update_movie(movie_id: int, patch: MoviePatch):
    for movie in movies:
        if movie["id"] == movie_id:
            updates = patch.dict(exclude_unset=True)
            movie.update(updates)
            return movie
    raise HTTPException(status_code=404, detail="Movie not found")
```

When you're ready, click **Run Tests**.
