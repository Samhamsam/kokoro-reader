FROM golang:1.26-bookworm AS builder

WORKDIR /build
COPY go.mod go.sum ./
RUN go mod download
COPY . .
RUN CGO_ENABLED=1 go build -o kokoro-server .

# Find sherpa-onnx shared libraries from Go module cache
RUN mkdir -p /sherpa-libs && \
    find /go/pkg/mod/github.com/k2-fsa -name "libsherpa-onnx-c-api.so" -path "*/x86_64*" -exec cp {} /sherpa-libs/ \; && \
    find /go/pkg/mod/github.com/k2-fsa -name "libonnxruntime.so" -path "*/x86_64*" -exec cp {} /sherpa-libs/ \;

FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates bzip2 && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /build/kokoro-server .
COPY --from=builder /sherpa-libs/*.so /usr/lib/

# Models mounted at runtime: -v /path/to/models-tts:/app/models-tts
# Data persisted via volume: -v kokoro-data:/app/data

EXPOSE 8787

ENTRYPOINT ["./kokoro-server", "models-tts", "data"]
