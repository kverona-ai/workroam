# workroam-server 容器镜像。
#
# 发布流水线把已编译好的 musl 静态二进制放到 dist/container/<amd64|arm64>/ 再构建，
# 镜像里不编译任何东西：
#   docker buildx build --platform linux/amd64,linux/arm64 -f deploy/server/Dockerfile .
#
# 运行（中继模式）：
#   docker run -p 8787:8787 -v workroam-server:/home/nonroot/.workroam-server \
#     ghcr.io/kverona-ai/workroam-server --listen 0.0.0.0:8787 --public-url https://relay.example.com
FROM gcr.io/distroless/static-debian12:nonroot

ARG TARGETARCH
COPY --chmod=0755 dist/container/${TARGETARCH}/workroam-server /usr/local/bin/workroam-server

LABEL org.opencontainers.image.title="workroam-server" \
      org.opencontainers.image.description="Optional relay and server for Workroam" \
      org.opencontainers.image.source="https://github.com/kverona-ai/workroam" \
      org.opencontainers.image.licenses="Apache-2.0"

USER nonroot
EXPOSE 8787
VOLUME ["/home/nonroot/.workroam-server"]
ENTRYPOINT ["/usr/local/bin/workroam-server"]
CMD ["--listen", "0.0.0.0:8787"]
