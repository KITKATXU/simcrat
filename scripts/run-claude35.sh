#!/bin/bash

# $1: num signatures
# $2: option

if [ $# -ne 2 ]; then
  exit 1
fi

function run(){
  cargo run --release -- --no-stage -q $@
}

OPT=" --log-file logfile.log --max-tokens 4096  -a /home/qxxu/tymcrat/.anthropic_api_key --model claude-3-5-sonnet-20240620  --show-openai-stat --show-program-size --real-time --show-time "

if [ $1 -eq 0 ]; then
  OPT+="--no-candidate "
else
  OPT+="--num-signatures $1 "
fi

if [ $2 -eq 1 ]; then
  OPT+="--no-fix "
elif [ $2 -eq 2 ]; then
  OPT+="--no-augmentation "
elif [ $2 -eq 3 ]; then
  OPT+="--no-augmentation --no-fix "
fi

run $OPT --output gzip.rs ~/gzip-1.12.json
