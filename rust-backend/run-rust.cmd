docker build -t beacon-wms-docker-rust-backend:dev -f Dockerfile.dev .

@REM remove container beacon-wms-docker-rust-backend-dev if it exists:
docker rm -f beacon-wms-docker-rust-backend-dev 2>nul

docker run -p 8080:8080 --name beacon-wms-docker-rust-backend-dev beacon-wms-docker-rust-backend:dev