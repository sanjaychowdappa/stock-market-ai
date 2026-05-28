FROM node:20-alpine

WORKDIR /app

COPY package.json package-lock.json* ./
RUN npm install

COPY . .

EXPOSE 3000

# Disable React dev error overlay — prevents benign ResizeObserver warnings
# from showing a full-screen red error page
ENV WDS_SOCKET_HOST=0.0.0.0
ENV REACT_APP_SUPPRESS_OVERLAY=true

CMD ["npm", "start"]
