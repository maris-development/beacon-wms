# ---- Build stage ----
FROM node:20-slim AS builder

WORKDIR /app

# Install dependencies
COPY package.json package-lock.json* ./
RUN npm ci 

# Copy the rest of the source
COPY . .

# Build (assuming you have a "build" script in package.json)
RUN npm run build


# ---- Runtime stage ----
FROM node:20-slim

WORKDIR /app

# Copy only necessary files from builder
COPY package.json package-lock.json* ./
RUN npm ci --omit=dev

COPY --from=builder /app/dist ./dist
COPY --from=builder /app/templates ./templates


# Run app
CMD ["node", "dist/index.js"]
