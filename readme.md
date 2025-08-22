
## list the available tools
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' | cargo run

### pretty print
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' | cargo run | jq

## read a file
echo "hello ai bot" > test.txt

echo '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"read_file","arguments":{"path":"test.txt"}}}' | cargo run


## write a file
echo '{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"write_file","arguments":{ "path":"test_output.txt","content":"Hello from the bot!","append":false}}}' | cargo run | jq

### append 
echo '{ "jsonrpc":"2.0", "id":11, "method":"tools/call", "params":{ "name":"write_file", "arguments":{ "path":"test_output.txt", "content":" Appending more text...", "append":true } } }' | cargo run | jq

## run an unshare isolated process
echo '{ "jsonrpc": "2.0", "id": 42, "method": "tools/call", "params": { "name": "unshare_exec", "arguments": { "binary": "/bin/ls", "args": ["-la"] } } }' | cargo run | jq