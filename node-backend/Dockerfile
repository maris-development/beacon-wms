# ---- Build stage ----
FROM node:20-slim AS builder

WORKDIR /app

# Install dependencies
COPY package.json package-lock.json* ./
RUN npm ci --omit=dev

# Copy the rest of the source
COPY . .

# Build (assuming you have a "build" script in package.json)
RUN npm run build


# ---- Runtime stage ----
FROM node:20-slim

WORKDIR /app

# Copy only necessary files from builder
COPY --from=builder /app/package.json ./ 
COPY --from=builder /app/node_modules ./node_modules
COPY --from=builder /app/dist ./dist
COPY --from=builder /app/templates ./templates


# Run app
CMD ["node", "dist/index.js"]
