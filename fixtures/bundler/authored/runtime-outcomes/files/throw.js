console.log('before throw');
throw Object.assign(new TypeError('authored throw'), {code: 'AUTHORED_FAILURE'});
