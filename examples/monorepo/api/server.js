const http = require('node:http');
http.createServer((_request, response) => response.end('API is healthy\n'))
  .listen(Number(process.env.PORT));
