# How to test beacon WMS (visually)

## First start beacon WMS:

You can do this by running `docker compose up -d` in the root.

Or running it manually (open two terminals):
```
cd node-backend
yarn run dev

```

```
cd rust-backend
cargo run --release 
```

## Start a dev server

Start a dev webserver in the `/test/` directory by running one of the commands below:

```bash
php -S localhost:8080
npx serve .
npx http-server . -p 8080
yarn dlx serve -p 8080
```

Then open [http://localhost:8080/index.html](http://localhost:8080/index.html)