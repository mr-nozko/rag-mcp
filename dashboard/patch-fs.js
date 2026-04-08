/**
 * patch-fs.js
 *
 * Windows NTFS Junction Readlink Fix
 * -----------------------------------
 * On Windows, when a file exists inside a path that contains an NTFS junction
 * (common with Anaconda/conda environments), calling fs.readlink() on a regular
 * FILE returns EISDIR instead of the correct EINVAL.
 *
 * Node.js's module resolver expects EINVAL to mean "not a symlink, skip it."
 * When it gets EISDIR instead, it throws an unhandled error.
 *
 * This patch translates EISDIR → EINVAL for readlink calls so that all of
 * Node.js's module resolution (and Next.js's page data workers) handle the
 * case correctly.
 *
 * Applied via NODE_OPTIONS=--require ./patch-fs.js
 */

const fs = require('fs');

// Patch async fs.readlink
const _readlink = fs.readlink.bind(fs);
fs.readlink = function (path, options, callback) {
  if (typeof options === 'function') {
    callback = options;
    options = {};
  }
  _readlink(path, options, function (err, linkString) {
    if (err && err.code === 'EISDIR') {
      const patched = new Error(`EINVAL: invalid argument, readlink '${path}'`);
      patched.code = 'EINVAL';
      patched.syscall = 'readlink';
      patched.path = path;
      return callback(patched);
    }
    callback(err, linkString);
  });
};

// Patch sync fs.readlinkSync
const _readlinkSync = fs.readlinkSync.bind(fs);
fs.readlinkSync = function (path, options) {
  try {
    return _readlinkSync(path, options);
  } catch (err) {
    if (err.code === 'EISDIR') {
      const patched = new Error(`EINVAL: invalid argument, readlink '${path}'`);
      patched.code = 'EINVAL';
      patched.syscall = 'readlink';
      patched.path = path;
      throw patched;
    }
    throw err;
  }
};

// Patch fs.promises.readlink
const _readlinkAsync = fs.promises.readlink.bind(fs.promises);
fs.promises.readlink = async function (path, options) {
  try {
    return await _readlinkAsync(path, options);
  } catch (err) {
    if (err.code === 'EISDIR') {
      const patched = new Error(`EINVAL: invalid argument, readlink '${path}'`);
      patched.code = 'EINVAL';
      patched.syscall = 'readlink';
      patched.path = path;
      throw patched;
    }
    throw err;
  }
};
