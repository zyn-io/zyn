#!/usr/bin/env bash
# Stingers synthesised with FFmpeg (no samples), then mixed onto the music at the cue times.
set -euo pipefail
cd "$(dirname "$0")"
A="-v error -y"
ffmpeg $A -f lavfi -i "aevalsrc='0.9*sin(2*PI*52*t)*exp(-5*t)+0.35*sin(2*PI*104*t)*exp(-9*t)':s=48000:d=0.9" impact-sub.wav
ffmpeg $A -f lavfi -i "anoisesrc=color=pink:amplitude=0.6:d=0.5:r=48000" -af "afade=t=out:st=0:d=0.5:curve=exp,highpass=f=600" impact-air.wav
ffmpeg $A -i impact-sub.wav -i impact-air.wav -filter_complex "amix=inputs=2:normalize=0" impact.wav
for f in 392 494 587; do
  ffmpeg $A -f lavfi -i "aevalsrc='0.5*(sin(2*PI*$f*t)+0.35*sin(2*PI*2*$f*t)+0.15*sin(2*PI*3*$f*t))*exp(-7*t)':s=48000:d=0.8" pluck-$f.wav
done
ffmpeg $A -f lavfi -i "aevalsrc='0.9*sin(2*PI*68*t)*exp(-11*t)':s=48000:d=0.5" thud-low.wav
ffmpeg $A -f lavfi -i "anoisesrc=color=white:amplitude=0.5:d=0.08:r=48000" -af "afade=t=out:st=0:d=0.08,lowpass=f=3000" thud-click.wav
ffmpeg $A -i thud-low.wav -i thud-click.wav -filter_complex "amix=inputs=2:normalize=0" thud.wav
ffmpeg $A -f lavfi -i "anoisesrc=color=pink:amplitude=0.5:d=0.6:r=48000" -af "volume='pow(t/0.6,2.5)':eval=frame,highpass=f=300,lowpass=f=6000" riser.wav
ffmpeg $A -f lavfi -i "anoisesrc=color=white:amplitude=0.35:d=0.5:r=48000" -af "volume='if(lt(mod(t,0.13),0.03),1,0)':eval=frame,bandpass=f=2500:w=1800" flicker.wav
ffmpeg $A -f lavfi -i "aevalsrc='0.6*(sin(2*PI*98*t)+0.4*sin(2*PI*196*t))*exp(-2.2*t)':s=48000:d=3" end-pluck.wav

# cue: file@seconds@gain
CUES="impact.wav@1.97@0.9 pluck-392.wav@2.45@1.4 pluck-494.wav@3.45@1.4 pluck-587.wav@4.45@1.4 thud.wav@5.98@0.7 impact-air.wav@5.95@0.6 thud.wav@10.97@0.9 riser.wav@11.4@0.45 impact.wav@11.98@1.0 flicker.wav@15.75@0.4 end-pluck.wav@17.0@0.6"
IN=(-i ../music.mp3); F=""; L="[0]volume='0.85*if(between(t,15.0,17.0),0.5,1)*if(between(t,11.0,12.0),0.75,1)*if(between(t,2.43,2.6)+between(t,3.43,3.6)+between(t,4.43,4.6),0.7,1)':eval=frame[m];"; i=1; MIX="[m]"
for c in $CUES; do
  IFS=@ read -r f t g <<<"$c"; IN+=(-i "$f")
  ms=$(python3 -c "print(int(round($t*1000)))")
  L+="[$i]volume=$g,adelay=${ms}|${ms},apad=whole_dur=20[s$i];"; MIX+="[s$i]"; i=$((i+1))
done
ffmpeg $A "${IN[@]}" -filter_complex "${L}${MIX}amix=inputs=$i:normalize=0:duration=first,atrim=0:20,loudnorm=I=-14:TP=-2.0:LRA=9[o]" -map "[o]" -ar 48000 -b:a 256k ../mix.mp3
