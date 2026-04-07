FROM golang:1.26-bookworm AS builder

WORKDIR /build
COPY go.mod go.sum ./
RUN go mod download
COPY . .
RUN CGO_ENABLED=1 go build -o kokoro-server .

FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates bzip2 && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /build/kokoro-server .

# Models mounted at runtime: -v /path/to/models-tts:/app/models-tts
# Data persisted via volume: -v kokoro-data:/app/data

EXPOSE 8787

ENTRYPOINT ["./kokoro-server", "models-tts", "data"]
