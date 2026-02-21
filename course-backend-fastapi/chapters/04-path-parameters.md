# Chapter 4: Path Parameters — Getting a Single Movie

You'll add a `GET /movies/{id}` endpoint that returns a single movie by its ID.

## What you'll build

- `GET /movies/{id}` returns the movie with that ID (200)
- Returns 404 if the movie doesn't exist

## Implementation

```python
from fastapi import FastAPI, HTTPException
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

@app.get("/movies/{movie_id}")
def get_movie(movie_id: int):
    for movie in movies:
        if movie["id"] == movie_id:
            return movie
    raise HTTPException(status_code=404, detail="Movie not found")
```

When you're ready, click **Run Tests**.
