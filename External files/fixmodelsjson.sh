#!/bin/sh

# Convert underscores to spaces in param names

# temporarily replace HD2_ with HD2~ so we can convert rest of offending underscores
# Other special cases
# HD2_
# Stereo1_2
# Stereo3_4
# EnvelopeADSR_
# HelixPlugin_
sed -e 's/HD2_/HD2~/g' $1 > $2
sed -ie 's/Stereo1_2/Stereo1~2/g' $2
sed -ie 's/Stereo3_4/Stereo3~4/g' $2
sed -ie 's/EnvelopeADSR_/EnvelopeADSR~/g' $2
sed -ie 's/HelixPlugin_/HelixPlugin~/g' $2

# change underscores to spaces
sed -ie 's/_/ /g' $2

# change special cases back
sed -ie 's/HD2~/HD2_/g' $2
sed -ie 's/Stereo1~2/Stereo1_2/g' $2
sed -ie 's/Stereo3~4/Stereo3_4/g' $2
sed -ie 's/EnvelopeADSR~/EnvelopeADSR_/g' $2
sed -ie 's/HelixPlugin~/HelixPlugin_/g' $2

# Change HD2_DelayVintageDigitalMonoV2 to HD2_DelayVintageDigitalV2Mono
sed -ie 's/HD2_DelayVintageDigitalMonoV2/HD2_DelayVintageDigitalV2Mono/g' $2

# Change HD2_DelayVintageDigitalStereoV2 to HD2_DelayVintageDigitalV2Stereo
sed -ie 's/HD2_DelayVintageDigitalStereoV2/HD2_DelayVintageDigitalV2Stereo/g' $2

