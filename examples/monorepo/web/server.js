const http = require('node:http');
http.createServer((_request, response) => response.end('Web is healthy\n'))
  .listen(Number(process.env.PORT));
