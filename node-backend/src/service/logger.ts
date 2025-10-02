import { createLogger, format, transports } from 'winston';
import DailyRotateFile from 'winston-daily-rotate-file';
import * as fs from 'fs';
import path from "path";

const logDir = process.env.LOG_DIR || '../logs';

//normalize log directory path
const normalizedLogDir = path.resolve(logDir);

fs.mkdirSync(normalizedLogDir, { recursive: true });

const logger = createLogger({

  level: 'info',

  format: format.combine(
    format.timestamp({ format: 'YYYY-MM-DD HH:mm:ss' }),
    format.printf(({ timestamp, level, message }) =>
      `${timestamp} [${level.toUpperCase()}] ${message}`
    )
  ),



  transports: [
    new transports.Console(),
    new DailyRotateFile({
      dirname: normalizedLogDir,
      filename: 'node-%DATE%.log',
      datePattern: 'YYYY-MM-DD',
      zippedArchive: true,
      maxSize: '20m',
      maxFiles: '14d'
    })
  ]
  
});

logger.info('Logger initialized, logging to directory: ' + normalizedLogDir);

export default logger;
